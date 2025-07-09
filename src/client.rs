use crate::{
    error::Error,
    header::HttpHeader,
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
const MAX_HEADERS: usize = 16;

macro_rules! try_push {
    ($expr:expr) => {
        if $expr.is_err() {
            return Err(Error::InvalidResponse("Request buffer overflow"));
        }
    };
}

/// HTTP Client for making HTTP requests with true zero-copy response handling
///
/// This is the main client struct for making HTTP requests. It provides methods
/// for performing GET, POST, PUT, DELETE and other HTTP requests using a zero-copy
/// approach where all response data is borrowed directly from user-provided buffers.
///
/// The client is designed to work with Embassy's networking stack and requires
/// users to provide their own response buffers, ensuring maximum memory efficiency
/// and control while maintaining `no_std` compatibility.
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

    /// Make an HTTP request with zero-copy response handling
    ///
    /// This is the core method for making HTTP requests using zero-copy approach.
    /// The caller provides a buffer where the response will be stored, and the
    /// returned `HttpResponse` will contain references to data within that buffer.
    ///
    /// # Arguments
    ///
    /// * `method` - The HTTP method to use (GET, POST, etc.)
    /// * `endpoint` - The URL to request (e.g., <http://example.com/api>)
    /// * `headers` - A slice of HTTP headers to include in the request
    /// * `body` - Optional request body data (required for POST/PUT requests)
    /// * `response_buffer` - A mutable buffer to store the response data
    ///
    /// # Returns
    ///
    /// * `Ok((HttpResponse, usize))` - Response with zero-copy body and bytes read
    /// * `Err(Error)` - Error occurred during the request process
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// * The URL is malformed or cannot be parsed
    /// * DNS resolution fails for the hostname
    /// * Network connection cannot be established
    /// * The request times out
    /// * The response cannot be parsed
    /// * The response buffer is too small for the response data
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use nanofish::{HttpClient, HttpHeader, HttpMethod, ResponseBody};
    /// use embassy_net::Stack;
    ///
    /// async fn example(client: &HttpClient<'_>) -> Result<(), nanofish::Error> {
    ///     let mut buffer = [0u8; 8192]; // You control the buffer size!
    ///     let (response, bytes_read) = client.request(
    ///         HttpMethod::GET,
    ///         "https://example.com",
    ///         &[],
    ///         None,
    ///         &mut buffer
    ///     ).await?;
    ///     
    ///     // Response body now contains direct references to data in buffer
    ///     match response.body {
    ///         ResponseBody::Text(text) => println!("Text: {}", text),
    ///         ResponseBody::Binary(bytes) => println!("Binary: {} bytes", bytes.len()),
    ///         ResponseBody::Empty => println!("Empty response"),
    ///     }
    ///     Ok(())
    /// }
    /// ```
    pub async fn request<'b>(
        &self,
        method: HttpMethod,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        body: Option<&[u8]>,
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error> {
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

        let total_read = match scheme {
            #[cfg(feature = "tls")]
            "https" => {
                self.make_https_request(method, (host, port), path, headers, body, response_buffer)
                    .await?
            }
            #[cfg(not(feature = "tls"))]
            "https" => return Err(Error::UnsupportedScheme("https (TLS support not enabled)")),
            "http" => {
                self.make_http_request(method, (host, port), path, headers, body, response_buffer)
                    .await?
            }
            _ => return Err(Error::UnsupportedScheme(scheme)),
        };

        let response = Self::parse_http_response_zero_copy(&response_buffer[..total_read])?;
        Ok((response, total_read))
    }

    /// Make HTTPS request over TLS with zero-copy response handling
    #[cfg(feature = "tls")]
    async fn make_https_request(
        &self,
        method: HttpMethod,
        host_port: (&str, u16),
        path: &str,
        headers: &[HttpHeader<'_>],
        body: Option<&[u8]>,
        response_buffer: &mut [u8],
    ) -> Result<usize, Error> {
        let (host, port) = host_port;
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
        }

        tls.flush().await?;

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

        Ok(total_read)
    }

    /// Make HTTP request with zero-copy response handling
    async fn make_http_request(
        &self,
        method: HttpMethod,
        host_port: (&str, u16),
        path: &str,
        headers: &[HttpHeader<'_>],
        body: Option<&[u8]>,
        response_buffer: &mut [u8],
    ) -> Result<usize, Error> {
        let (host, port) = host_port;
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
                    error!("Socket read error: {:?}", defmt::Debug2Format(&e));
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

        Ok(total_read)
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
    pub async fn patch<'b>(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        body: &[u8],
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error> {
        self.request(
            HttpMethod::PATCH,
            endpoint,
            headers,
            Some(body),
            response_buffer,
        )
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
    pub async fn head<'b>(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error> {
        self.request(HttpMethod::HEAD, endpoint, headers, None, response_buffer)
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
    pub async fn options<'b>(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error> {
        self.request(
            HttpMethod::OPTIONS,
            endpoint,
            headers,
            None,
            response_buffer,
        )
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
    pub async fn trace<'b>(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error> {
        self.request(HttpMethod::TRACE, endpoint, headers, None, response_buffer)
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
    pub async fn connect<'b>(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error> {
        self.request(
            HttpMethod::CONNECT,
            endpoint,
            headers,
            None,
            response_buffer,
        )
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
    pub async fn get<'b>(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error> {
        self.request(HttpMethod::GET, endpoint, headers, None, response_buffer)
            .await
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
    pub async fn post<'b>(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        body: &[u8],
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error> {
        self.request(
            HttpMethod::POST,
            endpoint,
            headers,
            Some(body),
            response_buffer,
        )
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
    pub async fn put<'b>(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        body: &[u8],
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error> {
        self.request(
            HttpMethod::PUT,
            endpoint,
            headers,
            Some(body),
            response_buffer,
        )
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
    pub async fn delete<'b>(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error> {
        self.request(HttpMethod::DELETE, endpoint, headers, None, response_buffer)
            .await
    }

    /// Parse HTTP response from raw data with zero-copy handling
    fn parse_http_response_zero_copy(data: &[u8]) -> Result<HttpResponse<'_>, Error> {
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
        let mut headers = Vec::<HttpHeader<'_>, MAX_HEADERS>::new();

        for header_line in headers_section.split("\r\n") {
            if let Some(colon_pos) = header_line.find(':') {
                let name = header_line[..colon_pos].trim();
                let value = header_line[colon_pos + 1..].trim();

                let header = HttpHeader::new(name, value);
                if headers.push(header).is_err() {
                    break;
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

    /// Parse response body based on content type and data (zero-copy)
    fn parse_response_body<'b>(
        headers: &[HttpHeader<'_>],
        body_data: &'b [u8],
    ) -> ResponseBody<'b> {
        if body_data.is_empty() {
            return ResponseBody::Empty;
        }

        // Check content type to determine how to handle the body
        if let Some(content_type) = Self::get_content_type(headers) {
            if Self::is_text_content_type(content_type) {
                Self::parse_as_text_or_binary(body_data)
            } else {
                ResponseBody::Binary(body_data)
            }
        } else {
            // No content type header, try to guess based on UTF-8 validity
            Self::parse_as_text_or_binary(body_data)
        }
    }

    /// Get content type from headers
    fn get_content_type<'h>(headers: &'h [HttpHeader<'_>]) -> Option<&'h str> {
        headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("Content-Type"))
            .map(|h| h.value)
    }

    /// Check if content type indicates text content
    fn is_text_content_type(content_type: &str) -> bool {
        content_type.starts_with("text/")
            || content_type.starts_with("application/json")
            || content_type.starts_with("application/xml")
            || content_type.starts_with("application/x-www-form-urlencoded")
    }

    /// Try to parse as text, fall back to binary if not valid UTF-8
    fn parse_as_text_or_binary(body_data: &[u8]) -> ResponseBody<'_> {
        if let Ok(text) = core::str::from_utf8(body_data) {
            ResponseBody::Text(text)
        } else {
            Self::parse_as_binary(body_data)
        }
    }

    /// Parse data as binary (zero-copy)
    fn parse_as_binary(body_data: &[u8]) -> ResponseBody<'_> {
        ResponseBody::Binary(body_data)
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
}

#[cfg(feature = "tls")]
fn timeseed() -> [u8; 32] {
    let bytes: [u8; 8] = Instant::now().as_ticks().to_be_bytes();
    let mut result: [u8; 32] = [0; 32];
    result[..8].copy_from_slice(&bytes);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::HttpClientOptions;
    use embassy_net::Stack;

    #[test]
    fn test_is_response_complete_headers_only() {
        let data = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\n";
        assert!(HttpClient::is_response_complete(data));
    }

    #[test]
    fn test_is_response_complete_with_content_length() {
        let data = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
        assert!(HttpClient::is_response_complete(data));
    }

    #[test]
    fn test_is_response_complete_incomplete() {
        let data = b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nshort";
        assert!(!HttpClient::is_response_complete(data));
    }

    #[test]
    fn test_new_and_with_options() {
        // This test only checks that the options are set correctly, not that the stack is valid.
        // Use a raw pointer to avoid UB and static mut issues. This is safe for type-checking only.
        let fake_stack: *const Stack = core::ptr::NonNull::dangling().as_ptr();
        let client = HttpClient::new(unsafe { &*fake_stack });
        let opts = HttpClientOptions {
            max_retries: 1,
            socket_timeout: embassy_time::Duration::from_secs(1),
            retry_delay: embassy_time::Duration::from_millis(1),
            socket_close_delay: embassy_time::Duration::from_millis(1),
        };
        let client2 = HttpClient::with_options(unsafe { &*fake_stack }, opts);
        assert_eq!(client.options.max_retries, 5);
        assert_eq!(client2.options.max_retries, 1);
    }
}
