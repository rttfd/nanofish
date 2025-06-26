use crate::{
    error::Error,
    header::{HttpHeader, OwnedHttpHeader},
    method::HttpMethod,
    options::HttpClientOptions,
    response::{HttpResponse, ResponseBody},
};
#[cfg(feature = "tls")]
use defmt::debug;
use defmt::error;
use embassy_net::{
    Stack,
    dns::{self, DnsSocket},
    tcp::TcpSocket,
};
#[cfg(feature = "tls")]
use embassy_time::Instant;
use embassy_time::Timer;
use embedded_io_async::Write as EmbeddedWrite;
#[cfg(feature = "tls")]
use embedded_tls::{Aes128GcmSha256, NoVerify, TlsConfig, TlsConnection, TlsContext};
use heapless::Vec;
#[cfg(feature = "tls")]
use rand_chacha::ChaCha8Rng;
#[cfg(feature = "tls")]
use rand_core::SeedableRng;

// Buffer sizes remain as compile-time constants
const REQUEST_SIZE: usize = 1024;
const TRANSMIT_BUFFER_SIZE: usize = 4096;
const RECEIVE_BUFFER_SIZE: usize = 4096;
const RESPONSE_BUFFER_SIZE: usize = 4096;
const RESPONSE_SIZE: usize = 2048;
const MAX_HEADERS: usize = 16;

macro_rules! try_push {
    ($expr:expr) => {
        if $expr.is_err() {
            return Err(Error::InvalidResponse("Request buffer overflow"));
        }
    };
}

/// HTTP Client for making HTTP requests
///
/// This is the main client struct for making HTTP requests. It provides methods
/// for performing GET, POST, PUT, DELETE and other HTTP requests.
///
/// The client is designed to work with Embassy's networking stack and
/// uses fixed-size buffers for all operations to maintain `no_std` compatibility.
pub struct HttpClient<'a> {
    /// Reference to the Embassy network stack
    stack: &'a Stack<'a>,
    /// HTTP client options
    options: HttpClientOptions,
}

impl<'a> HttpClient<'a> {
    /// Create a new HTTP client with default options
    #[must_use]
    pub fn new(stack: &'a Stack<'a>) -> Self {
        Self {
            stack,
            options: HttpClientOptions::default(),
        }
    }

    /// Create a new HTTP client with custom options
    #[must_use]
    pub fn with_options(stack: &'a Stack<'a>, options: HttpClientOptions) -> Self {
        Self { stack, options }
    }

    /// Make an HTTP request
    ///
    /// This is the core method for making HTTP requests. It handles all HTTP methods
    /// and manages the entire request flow, from DNS resolution to parsing the response.
    ///
    /// # Arguments
    ///
    /// * `method` - The HTTP method to use (GET, POST, etc.)
    /// * `endpoint` - The URL to request (e.g., <http://example.com/api>)
    /// * `headers` - A slice of HTTP headers to include in the request
    /// * `body` - Optional request body data (required for POST/PUT requests)
    ///
    /// # Returns
    ///
    /// * `Ok(HttpResponse)` - Successful response with status code, headers, and body
    /// * `Err(Error)` - Error occurred during the request process
    ///
    /// # Errors
    ///
    /// This function can return errors for various reasons including:
    /// - Network connectivity issues
    /// - DNS resolution failures
    /// - Invalid URLs or unsupported schemes
    /// - Connection timeouts
    /// - Invalid server responses
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use nanofish::{HttpClient, HttpHeader, HttpMethod};
    /// use embassy_net::Stack;
    ///
    /// async fn example(client: &HttpClient<'_>) -> Result<(), nanofish::Error> {
    ///     // Making a simple GET request
    ///     let response = client.request(HttpMethod::GET, "https://example.com", &[], None).await?;
    ///
    ///     // Making a POST request with JSON data
    ///     let json = b"{\"key\": \"value\"}";
    ///     let headers = [HttpHeader { name: "Content-Type", value: "application/json" }];
    ///     let response = client.request(HttpMethod::POST, "https://api.example.com/data", &headers, Some(json)).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn request(
        &self,
        method: HttpMethod,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        body: Option<&[u8]>,
    ) -> Result<HttpResponse, Error> {
        let (scheme, host_port) = if let Some(rest) = endpoint.strip_prefix("http://") {
            ("http", rest)
        } else if let Some(rest) = endpoint.strip_prefix("https://") {
            ("https", rest)
        } else {
            return Err(Error::InvalidUrl);
        };

        let url_parts: Vec<&str, 8> = host_port.split('/').collect();
        if url_parts.is_empty() {
            return Err(Error::InvalidUrl);
        }

        let host = url_parts[0];
        let path = &host_port[host.len()..];

        let (host, port) = if let Some(colon_pos) = host.rfind(':') {
            if let Ok(port) = host[colon_pos + 1..].parse::<u16>() {
                (&host[..colon_pos], port)
            } else {
                (host, if scheme == "https" { 443 } else { 80 })
            }
        } else {
            (host, if scheme == "https" { 443 } else { 80 })
        };

        match scheme {
            #[cfg(feature = "tls")]
            "https" => {
                self.make_https_request(method, host, port, path, headers, body)
                    .await
            }
            #[cfg(not(feature = "tls"))]
            "https" => Err(Error::UnsupportedScheme("https (TLS support not enabled)")),
            "http" => {
                self.make_http_request(method, host, port, path, headers, body)
                    .await
            }
            _ => Err(Error::UnsupportedScheme(scheme)),
        }
    }

    /// Make HTTPS request over TLS
    #[cfg(feature = "tls")]
    async fn make_https_request(
        &self,
        method: HttpMethod,
        host: &str,
        port: u16,
        path: &str,
        headers: &[HttpHeader<'_>],
        body: Option<&[u8]>,
    ) -> Result<HttpResponse, Error> {
        let mut rx_buffer = [0; RECEIVE_BUFFER_SIZE];
        let mut tx_buffer = [0; TRANSMIT_BUFFER_SIZE];
        let mut socket = TcpSocket::new(*self.stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(self.options.socket_timeout));

        let dns_socket = DnsSocket::new(*self.stack);
        let ip_addresses = dns_socket.query(host, dns::DnsQueryType::A).await?;

        if ip_addresses.is_empty() {
            return Err(Error::IpAddressEmpty);
        }

        let ip_addr = ip_addresses[0];
        let remote_endpoint = (ip_addr, port);

        socket
            .connect(remote_endpoint)
            .await
            .map_err(|e: embassy_net::tcp::ConnectError| {
                socket.abort();
                Error::from(e)
            })?;

        let mut read_record_buffer = [0; 16384];
        let mut write_record_buffer = [0; 16384];

        let tls_config: TlsConfig<'_, Aes128GcmSha256> = TlsConfig::new().with_server_name(host);
        let mut tls = TlsConnection::new(socket, &mut read_record_buffer, &mut write_record_buffer);
        let mut rng = ChaCha8Rng::from_seed(timeseed());

        tls.open::<_, NoVerify>(TlsContext::new(&tls_config, &mut rng))
            .await?;

        let http_request = Self::build_http_request(method, host, path, headers, body)?;

        tls.write_all(http_request.as_bytes()).await?;

        if let Some(body_data) = body {
            tls.write_all(body_data).await?;
        };

        tls.flush().await?;

        let mut response_buffer = [0u8; RESPONSE_BUFFER_SIZE];
        let mut total_read = 0;
        let mut retries = self.options.max_retries;

        while total_read < response_buffer.len() && retries > 0 {
            match tls.read(&mut response_buffer[total_read..]).await {
                Ok(0) => {
                    break;
                }
                Ok(n) => {
                    total_read += n;
                    if Self::is_response_complete(&response_buffer[..total_read]) {
                        break;
                    }
                }
                Err(e) => {
                    retries -= 1;
                    if retries > 0 {
                        Timer::after(self.options.retry_delay).await;
                    } else {
                        return Err(Error::TlsError(e));
                    }
                }
            }
        }

        if let Err((_, e)) = tls.close().await {
            debug!("Error closing TLS connection: {:?}", Error::from(e));
        }

        Timer::after(self.options.socket_close_delay).await;

        if total_read == 0 {
            return Err(Error::NoResponse);
        }

        Self::parse_http_response(&response_buffer[..total_read])
    }

    /// Make HTTP request over plain TCP
    async fn make_http_request(
        &self,
        method: HttpMethod,
        host: &str,
        port: u16,
        path: &str,
        headers: &[HttpHeader<'_>],
        body: Option<&[u8]>,
    ) -> Result<HttpResponse, Error> {
        let mut rx_buffer = [0; RECEIVE_BUFFER_SIZE];
        let mut tx_buffer = [0; TRANSMIT_BUFFER_SIZE];
        let mut socket = TcpSocket::new(*self.stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(self.options.socket_timeout));

        let dns_socket = DnsSocket::new(*self.stack);
        let ip_addresses = dns_socket.query(host, dns::DnsQueryType::A).await?;

        if ip_addresses.is_empty() {
            return Err(Error::IpAddressEmpty);
        }

        let ip_addr = ip_addresses[0];
        let remote_endpoint = (ip_addr, port);

        socket
            .connect(remote_endpoint)
            .await
            .map_err(|e: embassy_net::tcp::ConnectError| {
                socket.abort();
                Error::from(e)
            })?;

        let http_request = Self::build_http_request(method, host, path, headers, body)?;

        socket
            .write_all(http_request.as_bytes())
            .await
            .map_err(|e| {
                socket.abort();
                Error::from(e)
            })?;

        if let Some(body_data) = body {
            socket.write_all(body_data).await.map_err(|e| {
                socket.abort();
                Error::from(e)
            })?;
        }

        let mut response_buffer = [0u8; RESPONSE_BUFFER_SIZE];
        let mut total_read = 0;
        let mut retries = self.options.max_retries;

        while total_read < response_buffer.len() && retries > 0 {
            match socket.read(&mut response_buffer[total_read..]).await {
                Ok(0) => {
                    break;
                }
                Ok(n) => {
                    total_read += n;
                    if Self::is_response_complete(&response_buffer[..total_read]) {
                        break;
                    }
                }
                Err(e) => {
                    error!("Socket read error: {:?}", e);
                    retries -= 1;
                    if retries > 0 {
                        Timer::after(self.options.retry_delay).await;
                    }
                }
            }
        }

        socket.close();
        Timer::after(self.options.socket_close_delay).await;

        if total_read == 0 {
            return Err(Error::NoResponse);
        }

        Self::parse_http_response(&response_buffer[..total_read])
    }

    /// Check if HTTP response is complete
    fn is_response_complete(data: &[u8]) -> bool {
        let response_str = core::str::from_utf8(data).unwrap_or_default();

        if !response_str.contains("\r\n\r\n") {
            return false;
        }

        // Check for Content-Length header to determine if we have the full body
        if let Some(content_length_pos) = response_str.find("Content-Length:") {
            let content_length_end = response_str[content_length_pos..]
                .find("\r\n")
                .unwrap_or_default()
                + content_length_pos;
            let content_length_str =
                &response_str[content_length_pos + 15..content_length_end].trim();

            if let Ok(content_length) = content_length_str.parse::<usize>() {
                let headers_end = response_str.find("\r\n\r\n").unwrap_or_default() + 4;
                let body_received = data.len().saturating_sub(headers_end);
                return body_received >= content_length;
            }
        }

        true
    }

    /// Parse HTTP response from raw data
    fn parse_http_response(data: &[u8]) -> Result<HttpResponse, Error> {
        let response_str = core::str::from_utf8(data)
            .map_err(|_| Error::InvalidResponse("Invalid HTTP response encoding"))?;

        let status_line_end = response_str
            .find("\r\n")
            .ok_or(Error::InvalidResponse("Invalid HTTP response format"))?;

        let status_line = &response_str[..status_line_end];
        let status_code_str = status_line
            .split_whitespace()
            .nth(1)
            .ok_or(Error::InvalidResponse("Invalid HTTP status line"))?;

        let status_code = status_code_str
            .parse::<u16>()
            .map_err(|_| Error::InvalidResponse("Invalid HTTP status code"))?;

        let headers_end = response_str
            .find("\r\n\r\n")
            .ok_or(Error::InvalidResponse("Invalid HTTP response format"))?
            + 4;

        let headers_section = &response_str[status_line_end + 2..headers_end - 4];
        let mut headers = Vec::<OwnedHttpHeader, MAX_HEADERS>::new();

        for header_line in headers_section.split("\r\n") {
            if let Some(colon_pos) = header_line.find(':') {
                let name = header_line[..colon_pos].trim();
                let value = header_line[colon_pos + 1..].trim();

                if let Ok(header) = OwnedHttpHeader::new(name, value) {
                    if headers.push(header).is_err() {
                        break;
                    }
                }
            }
        }

        let body_data = if headers_end < data.len() {
            &data[headers_end..]
        } else {
            &[]
        };

        // Determine response body type and content
        let body = Self::parse_response_body(&headers, body_data);

        Ok(HttpResponse {
            status_code,
            headers,
            body,
        })
    }

    /// Parse response body based on content type and data
    fn parse_response_body(headers: &[OwnedHttpHeader], body_data: &[u8]) -> ResponseBody {
        if body_data.is_empty() {
            return ResponseBody::Empty;
        }

        // Check content type to determine how to handle the body
        if let Some(content_type) = Self::get_content_type(headers) {
            if Self::is_text_content_type(content_type) {
                Self::parse_as_text_or_binary(body_data)
            } else {
                Self::parse_as_binary(body_data)
            }
        } else {
            // No content type header, try to guess based on UTF-8 validity
            Self::parse_as_text_or_binary(body_data)
        }
    }

    /// Get content type from headers
    fn get_content_type(headers: &[OwnedHttpHeader]) -> Option<&str> {
        headers
            .iter()
            .find(|h| h.name().eq_ignore_ascii_case("Content-Type"))
            .map(super::header::OwnedHttpHeader::value)
    }

    /// Check if content type indicates text content
    fn is_text_content_type(content_type: &str) -> bool {
        content_type.starts_with("text/")
            || content_type.starts_with("application/json")
            || content_type.starts_with("application/xml")
            || content_type.starts_with("application/x-www-form-urlencoded")
    }

    /// Try to parse as text, fall back to binary if not valid UTF-8
    fn parse_as_text_or_binary(body_data: &[u8]) -> ResponseBody {
        if let Ok(text) = core::str::from_utf8(body_data) {
            let mut body_string = heapless::String::<RESPONSE_SIZE>::new();
            let _ = body_string.push_str(text);
            ResponseBody::Text(body_string)
        } else {
            Self::parse_as_binary(body_data)
        }
    }

    /// Parse data as binary
    fn parse_as_binary(body_data: &[u8]) -> ResponseBody {
        let mut body_vec = heapless::Vec::<u8, RESPONSE_SIZE>::new();
        for &byte in body_data.iter().take(RESPONSE_SIZE) {
            if body_vec.push(byte).is_err() {
                break;
            }
        }
        ResponseBody::Binary(body_vec)
    }

    /// Build HTTP request string
    fn build_http_request(
        method: HttpMethod,
        host: &str,
        path: &str,
        headers: &[HttpHeader<'_>],
        body: Option<&[u8]>,
    ) -> Result<heapless::String<REQUEST_SIZE>, Error> {
        let mut http_request = heapless::String::<REQUEST_SIZE>::new();

        try_push!(http_request.push_str(method.as_str()));
        try_push!(http_request.push_str(" "));
        try_push!(http_request.push_str(path));
        try_push!(http_request.push_str(" HTTP/1.1\r\n"));
        try_push!(http_request.push_str("Host: "));
        try_push!(http_request.push_str(host));
        try_push!(http_request.push_str("\r\n"));

        let mut content_length_present = false;

        for header in headers {
            try_push!(http_request.push_str(header.name));
            try_push!(http_request.push_str(": "));
            try_push!(http_request.push_str(header.value));
            try_push!(http_request.push_str("\r\n"));

            if header.name.eq_ignore_ascii_case("Content-Length") {
                content_length_present = true;
            }
        }

        // Add Content-Length header if body is present and not already specified
        if !content_length_present && body.is_some() {
            try_push!(http_request.push_str("Content-Length: "));
            let mut len_str = heapless::String::<8>::new();
            if core::fmt::write(
                &mut len_str,
                format_args!("{}", body.unwrap_or_default().len()),
            )
            .is_err()
            {
                return Err(Error::InvalidResponse("Failed to write content length"));
            }
            try_push!(http_request.push_str(&len_str));
            try_push!(http_request.push_str("\r\n"));
        }

        try_push!(http_request.push_str("Connection: close\r\n"));
        try_push!(http_request.push_str("\r\n"));

        Ok(http_request)
    }

    /// Convenience method for making a PATCH request
    ///
    /// # Arguments
    /// * `endpoint` - The URL to request (e.g., <http://example.com/api>)
    /// * `headers` - A slice of HTTP headers to include in the request
    /// * `body` - The request body data
    ///
    /// # Returns
    /// * `Ok(HttpResponse)` - Successful response
    /// * `Err(Error)` - Error occurred during the request process
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpClient::request`].
    pub async fn patch(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        body: &[u8],
    ) -> Result<HttpResponse, Error> {
        self.request(HttpMethod::PATCH, endpoint, headers, Some(body))
            .await
    }

    /// Convenience method for making a HEAD request
    ///
    /// # Arguments
    /// * `endpoint` - The URL to request (e.g., <http://example.com/api>)
    /// * `headers` - A slice of HTTP headers to include in the request
    ///
    /// # Returns
    /// * `Ok(HttpResponse)` - Successful response
    /// * `Err(Error)` - Error occurred during the request process
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpClient::request`].
    pub async fn head(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
    ) -> Result<HttpResponse, Error> {
        self.request(HttpMethod::HEAD, endpoint, headers, None)
            .await
    }

    /// Convenience method for making an OPTIONS request
    ///
    /// # Arguments
    /// * `endpoint` - The URL to request (e.g., <http://example.com/api>)
    /// * `headers` - A slice of HTTP headers to include in the request
    ///
    /// # Returns
    /// * `Ok(HttpResponse)` - Successful response
    /// * `Err(Error)` - Error occurred during the request process
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpClient::request`].
    pub async fn options(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
    ) -> Result<HttpResponse, Error> {
        self.request(HttpMethod::OPTIONS, endpoint, headers, None)
            .await
    }

    /// Convenience method for making a TRACE request
    ///
    /// # Arguments
    /// * `endpoint` - The URL to request (e.g., <http://example.com/api>)
    /// * `headers` - A slice of HTTP headers to include in the request
    ///
    /// # Returns
    /// * `Ok(HttpResponse)` - Successful response
    /// * `Err(Error)` - Error occurred during the request process
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpClient::request`].
    pub async fn trace(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
    ) -> Result<HttpResponse, Error> {
        self.request(HttpMethod::TRACE, endpoint, headers, None)
            .await
    }

    /// Convenience method for making a CONNECT request
    ///
    /// # Arguments
    /// * `endpoint` - The URL to request (e.g., <http://example.com/api>)
    /// * `headers` - A slice of HTTP headers to include in the request
    ///
    /// # Returns
    /// * `Ok(HttpResponse)` - Successful response
    /// * `Err(Error)` - Error occurred during the request process
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpClient::request`].
    pub async fn connect(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
    ) -> Result<HttpResponse, Error> {
        self.request(HttpMethod::CONNECT, endpoint, headers, None)
            .await
    }

    /// Convenience method for making a GET request
    ///
    /// # Arguments
    /// * `endpoint` - The URL to request (e.g., <http://example.com/api>)
    /// * `headers` - A slice of HTTP headers to include in the request
    ///
    /// # Returns
    /// * `Ok(HttpResponse)` - Successful response
    /// * `Err(Error)` - Error occurred during the request process
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpClient::request`].
    pub async fn get(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
    ) -> Result<HttpResponse, Error> {
        self.request(HttpMethod::GET, endpoint, headers, None).await
    }

    /// Convenience method for making a POST request
    ///
    /// # Arguments
    /// * `endpoint` - The URL to request (e.g., <http://example.com/api>)
    /// * `headers` - A slice of HTTP headers to include in the request
    /// * `body` - The request body data
    ///
    /// # Returns
    /// * `Ok(HttpResponse)` - Successful response
    /// * `Err(Error)` - Error occurred during the request process
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpClient::request`].
    pub async fn post(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        body: &[u8],
    ) -> Result<HttpResponse, Error> {
        self.request(HttpMethod::POST, endpoint, headers, Some(body))
            .await
    }

    /// Convenience method for making a PUT request
    ///
    /// # Arguments
    /// * `endpoint` - The URL to request (e.g., <http://example.com/api>)
    /// * `headers` - A slice of HTTP headers to include in the request
    /// * `body` - The request body data
    ///
    /// # Returns
    /// * `Ok(HttpResponse)` - Successful response
    /// * `Err(Error)` - Error occurred during the request process
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpClient::request`].
    pub async fn put(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        body: &[u8],
    ) -> Result<HttpResponse, Error> {
        self.request(HttpMethod::PUT, endpoint, headers, Some(body))
            .await
    }

    /// Convenience method for making a DELETE request
    ///
    /// # Arguments
    /// * `endpoint` - The URL to request (e.g., <http://example.com/api>)
    /// * `headers` - A slice of HTTP headers to include in the request
    ///
    /// # Returns
    /// * `Ok(HttpResponse)` - Successful response
    /// * `Err(Error)` - Error occurred during the request process
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpClient::request`].
    pub async fn delete(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
    ) -> Result<HttpResponse, Error> {
        self.request(HttpMethod::DELETE, endpoint, headers, None)
            .await
    }
}

#[cfg(feature = "tls")]
fn timeseed() -> [u8; 32] {
    let bytes: [u8; 8] = Instant::now().as_ticks().to_be_bytes();
    let mut result: [u8; 32] = [0; 32];
    result[..8].copy_from_slice(&bytes);
    result
}
