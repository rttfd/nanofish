use crate::{
    error::Error,
    header::{
        HttpHeader,
        headers::{CONTENT_LENGTH, CONTENT_TYPE},
    },
    method::HttpMethod,
    options::HttpClientOptions,
    protocol::{
        self, CHUNKED, CHUNKED_END_MARKER, CONNECTION_CLOSE_END, CRLF_LEN, CRLF_STR,
        DEFAULT_HTTP_PORT, DEFAULT_HTTPS_PORT, DOUBLE_CRLF_LEN, HEADER_SEPARATOR,
        HTTP_VERSION_LINE_SUFFIX, MAX_HEADERS, TRANSFER_ENCODING,
    },
    response::{HttpResponse, ResponseBody},
    status_code::StatusCode,
};
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
use embedded_tls::{Aes128GcmSha256, TlsConfig, TlsConnection, TlsContext, UnsecureProvider};
use heapless::Vec;
#[cfg(feature = "tls")]
use rand_chacha::ChaCha8Rng;
#[cfg(feature = "tls")]
use rand_core::SeedableRng;

const REQUEST_SIZE: usize = 1024;
const SMALL_BUFFER_SIZE: usize = 1024;
const MEDIUM_BUFFER_SIZE: usize = 4096;

/// Type alias for `HttpClient` with default buffer sizes
pub type DefaultHttpClient<'a> = HttpClient<
    'a,
    MEDIUM_BUFFER_SIZE, // TCP_RX: 4KB
    MEDIUM_BUFFER_SIZE, // TCP_TX: 4KB
    MEDIUM_BUFFER_SIZE, // TLS_READ: 4KB
    MEDIUM_BUFFER_SIZE, // TLS_WRITE: 4KB
    REQUEST_SIZE,       // RQ: 1KB
>;

/// Type alias for `HttpClient` with small buffer sizes for memory-constrained environments
pub type SmallHttpClient<'a> = HttpClient<
    'a,
    SMALL_BUFFER_SIZE, // TCP_RX: 1KB
    SMALL_BUFFER_SIZE, // TCP_TX: 1KB
    SMALL_BUFFER_SIZE, // TLS_READ: 1KB
    SMALL_BUFFER_SIZE, // TLS_WRITE: 1KB
    REQUEST_SIZE,      // RQ: 1KB
>;

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
///
/// # Type Parameters
///
/// * `TCP_RX` - TCP receive buffer size (default: 4096 bytes)
/// * `TCP_TX` - TCP transmit buffer size (default: 4096 bytes)
/// * `TLS_READ` - TLS read record buffer size (default: 4096 bytes, when TLS feature is enabled)
/// * `TLS_WRITE` - TLS write record buffer size (default: 4096 bytes, when TLS feature is enabled)
/// * `RQ` - HTTP request buffer size for building requests (default: 1024 bytes)
pub struct HttpClient<
    'a,
    const TCP_RX: usize = MEDIUM_BUFFER_SIZE,
    const TCP_TX: usize = MEDIUM_BUFFER_SIZE,
    const TLS_READ: usize = MEDIUM_BUFFER_SIZE,
    const TLS_WRITE: usize = MEDIUM_BUFFER_SIZE,
    const RQ: usize = REQUEST_SIZE,
> {
    /// Reference to the Embassy network stack
    stack: &'a Stack<'a>,
    /// HTTP client options
    options: HttpClientOptions,
}

impl<
    'a,
    const TCP_RX: usize,
    const TCP_TX: usize,
    const TLS_READ: usize,
    const TLS_WRITE: usize,
    const RQ: usize,
> HttpClient<'a, TCP_RX, TCP_TX, TLS_READ, TLS_WRITE, RQ>
{
    /// Create a new HTTP client with custom buffer sizes and default options
    #[must_use]
    pub fn new(stack: &'a Stack<'a>) -> Self {
        Self {
            stack,
            options: HttpClientOptions::default(),
        }
    }

    /// Create a new HTTP client with custom buffer sizes and custom options
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
    /// use nanofish::{DefaultHttpClient, HttpHeader, HttpMethod, ResponseBody};
    /// use embassy_net::Stack;
    ///
    /// async fn example(stack: &Stack<'_>) -> Result<(), nanofish::Error> {
    ///     let client = DefaultHttpClient::new(stack);
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

        let host = host_port.split('/').next().ok_or(Error::InvalidUrl)?;
        let path = &host_port[host.len()..];
        let path = if path.is_empty() { "/" } else { path };

        let default_port = if scheme == "https" {
            DEFAULT_HTTPS_PORT
        } else {
            DEFAULT_HTTP_PORT
        };
        let (host, port) = if let Some(colon_pos) = host.rfind(':') {
            if let Ok(port) = host[colon_pos + 1..].parse::<u16>() {
                (&host[..colon_pos], port)
            } else {
                (host, default_port)
            }
        } else {
            (host, default_port)
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

        // Decode chunked transfer-encoding in-place if present
        let total_read = Self::dechunk(response_buffer, total_read)?;

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
        let mut rx_buffer = [0; TCP_RX];
        let mut tx_buffer = [0; TCP_TX];
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

        let mut read_record_buffer = [0; TLS_READ];
        let mut write_record_buffer = [0; TLS_WRITE];

        let tls_config = TlsConfig::new().with_server_name(host);
        let mut tls = TlsConnection::new(socket, &mut read_record_buffer, &mut write_record_buffer);
        let rng = ChaCha8Rng::from_seed(timeseed());

        tls.open(TlsContext::new(
            &tls_config,
            UnsecureProvider::new::<Aes128GcmSha256>(rng),
        ))
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
        let mut rx_buffer = [0; TCP_RX];
        let mut tx_buffer = [0; TCP_TX];
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
                    error!("Socket read error: {:?}", e);
                    retries -= 1;
                    if retries > 0 {
                        Timer::after(self.options.retry_delay).await;
                    } else {
                        socket.close();
                        return Err(Error::from(e));
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
        // Find the end of headers delimiter in raw bytes to avoid
        // requiring the entire response (including binary body) to be valid UTF-8.
        let headers_end = protocol::find_double_crlf(data)
            .ok_or(Error::InvalidResponse("Invalid HTTP response format"))?
            + DOUBLE_CRLF_LEN;

        let header_bytes = &data[..headers_end];
        let response_str = core::str::from_utf8(header_bytes)
            .map_err(|_| Error::InvalidResponse("Invalid HTTP response encoding"))?;

        let status_line_end = protocol::find_crlf(header_bytes)
            .ok_or(Error::InvalidResponse("Invalid HTTP response format"))?;

        let status_line = &response_str[..status_line_end];
        let status_code_str = status_line
            .split_whitespace()
            .nth(1)
            .ok_or(Error::InvalidResponse("Invalid HTTP status line"))?;

        let status_code: StatusCode = status_code_str.try_into()?;

        let headers_section =
            &response_str[status_line_end + CRLF_LEN..headers_end - DOUBLE_CRLF_LEN];
        let mut headers = Vec::<HttpHeader<'_>, MAX_HEADERS>::new();

        for header_line in headers_section.split(CRLF_STR) {
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
            .find(|h| h.name.eq_ignore_ascii_case(CONTENT_TYPE))
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
    ) -> Result<heapless::String<RQ>, Error> {
        let mut http_request = heapless::String::<RQ>::new();

        try_push!(http_request.push_str(method.as_str()));
        try_push!(http_request.push_str(" "));
        try_push!(http_request.push_str(path));
        try_push!(http_request.push_str(HTTP_VERSION_LINE_SUFFIX));
        try_push!(http_request.push_str("Host: "));
        try_push!(http_request.push_str(host));
        try_push!(http_request.push_str(CRLF_STR));

        let mut content_length_present = false;

        for header in headers {
            try_push!(http_request.push_str(header.name));
            try_push!(http_request.push_str(HEADER_SEPARATOR));
            try_push!(http_request.push_str(header.value));
            try_push!(http_request.push_str(CRLF_STR));

            if header.name.eq_ignore_ascii_case(CONTENT_LENGTH) {
                content_length_present = true;
            }
        }

        // Add Content-Length header if body is present and not already specified
        if !content_length_present && body.is_some() {
            try_push!(http_request.push_str(CONTENT_LENGTH));
            try_push!(http_request.push_str(HEADER_SEPARATOR));
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
            try_push!(http_request.push_str(CRLF_STR));
        }

        try_push!(http_request.push_str(CONNECTION_CLOSE_END));

        Ok(http_request)
    }

    /// Check if HTTP response is complete
    fn is_response_complete(data: &[u8]) -> bool {
        if protocol::find_double_crlf(data).is_none() {
            return false;
        }

        // Check for chunked transfer encoding
        if Self::has_chunked_transfer_encoding(data) {
            return data
                .windows(CHUNKED_END_MARKER.len())
                .any(|w| w == CHUNKED_END_MARKER);
        }

        // Check for Content-Length header to determine if we have the full body
        let headers_end = match protocol::find_double_crlf(data) {
            Some(pos) => pos + DOUBLE_CRLF_LEN,
            None => return true,
        };
        let header_bytes = &data[..headers_end];
        if let Ok(headers_str) = core::str::from_utf8(header_bytes)
            && let Some(value) = protocol::find_header_value(headers_str, CONTENT_LENGTH)
            && let Ok(content_length) = value.parse::<usize>()
        {
            let body_received = data.len().saturating_sub(headers_end);
            return body_received >= content_length;
        }

        true
    }

    /// Check if the response uses chunked transfer encoding
    fn has_chunked_transfer_encoding(data: &[u8]) -> bool {
        let headers_end = match protocol::find_double_crlf(data) {
            Some(pos) => pos + DOUBLE_CRLF_LEN,
            None => return false,
        };

        let header_bytes = &data[..headers_end];
        if let Ok(headers_str) = core::str::from_utf8(header_bytes)
            && let Some(value) = protocol::find_header_value(headers_str, TRANSFER_ENCODING)
        {
            return value.eq_ignore_ascii_case(CHUNKED);
        }
        false
    }

    /// Decode chunked transfer-encoding in-place if the response uses it.
    /// Returns the new total length after decoding.
    fn dechunk(buffer: &mut [u8], total_read: usize) -> Result<usize, Error> {
        let data = &buffer[..total_read];

        if !Self::has_chunked_transfer_encoding(data) {
            return Ok(total_read);
        }

        // Find end of headers
        let headers_end = protocol::find_double_crlf(data)
            .ok_or(Error::InvalidResponse("Invalid HTTP response format"))?
            + DOUBLE_CRLF_LEN;

        // Decode chunks in-place starting after headers
        let mut read_pos = headers_end;
        let mut write_pos = headers_end;

        while read_pos < total_read {
            // Find end of chunk size line
            let chunk_line_end = match protocol::find_crlf(&buffer[read_pos..total_read]) {
                Some(pos) => read_pos + pos,
                None => break,
            };

            // Parse chunk size (hex)
            let chunk_size_str = match core::str::from_utf8(&buffer[read_pos..chunk_line_end]) {
                Ok(s) => s.trim(),
                Err(_) => return Err(Error::InvalidResponse("Invalid chunk size encoding")),
            };

            // Chunk size may have extensions after a semicolon
            let size_part = chunk_size_str.split(';').next().unwrap_or("0").trim();
            let chunk_size = usize::from_str_radix(size_part, 16)
                .map_err(|_| Error::InvalidResponse("Invalid chunk size"))?;

            if chunk_size == 0 {
                // Final chunk
                break;
            }

            // Move past chunk size line (\r\n)
            let chunk_data_start = chunk_line_end + CRLF_LEN;
            let chunk_data_end = chunk_data_start + chunk_size;

            if chunk_data_end > total_read {
                return Err(Error::InvalidResponse("Incomplete chunked body"));
            }

            // Copy chunk data in-place (memmove semantics)
            if write_pos != chunk_data_start {
                buffer.copy_within(chunk_data_start..chunk_data_end, write_pos);
            }
            write_pos += chunk_size;

            // Skip past chunk data and trailing \r\n
            read_pos = chunk_data_end + CRLF_LEN;
        }

        Ok(write_pos)
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
    use embassy_net::Stack;

    #[test]
    fn test_is_response_complete_headers_only() {
        let data = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\n";
        assert!(DefaultHttpClient::is_response_complete(data));
    }

    #[test]
    fn test_is_response_complete_with_content_length() {
        let data = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
        assert!(DefaultHttpClient::is_response_complete(data));
    }

    #[test]
    fn test_is_response_complete_incomplete() {
        let data = b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nshort";
        assert!(!DefaultHttpClient::is_response_complete(data));
    }

    #[test]
    fn test_new_and_with_options() {
        // This test only checks that the options are set correctly, not that the stack is valid.
        // Use a raw pointer to avoid UB and static mut issues. This is safe for type-checking only.
        let fake_stack: *const Stack = core::ptr::NonNull::dangling().as_ptr();
        let client = DefaultHttpClient::new(unsafe { &*fake_stack });
        let opts = HttpClientOptions {
            max_retries: 1,
            socket_timeout: embassy_time::Duration::from_secs(1),
            retry_delay: embassy_time::Duration::from_millis(1),
            socket_close_delay: embassy_time::Duration::from_millis(1),
        };
        let client2 = DefaultHttpClient::with_options(unsafe { &*fake_stack }, opts);
        assert_eq!(client.options.max_retries, 5);
        assert_eq!(client2.options.max_retries, 1);
    }

    #[test]
    fn test_default_http_client_constructors() {
        let fake_stack: *const Stack = core::ptr::NonNull::dangling().as_ptr();
        let client_default = DefaultHttpClient::new(unsafe { &*fake_stack });
        assert_eq!(client_default.options.max_retries, 5);

        let client_custom = DefaultHttpClient::with_options(
            unsafe { &*fake_stack },
            HttpClientOptions {
                max_retries: 3,
                socket_timeout: embassy_time::Duration::from_secs(2),
                retry_delay: embassy_time::Duration::from_millis(10),
                socket_close_delay: embassy_time::Duration::from_millis(5),
            },
        );
        assert_eq!(client_custom.options.max_retries, 3);
    }

    #[test]
    fn test_small_http_client_constructors() {
        let fake_stack: *const Stack = core::ptr::NonNull::dangling().as_ptr();
        let client_small = SmallHttpClient::new(unsafe { &*fake_stack });
        assert_eq!(client_small.options.max_retries, 5);

        let client_small_custom = SmallHttpClient::with_options(
            unsafe { &*fake_stack },
            HttpClientOptions {
                max_retries: 2,
                socket_timeout: embassy_time::Duration::from_secs(1),
                retry_delay: embassy_time::Duration::from_millis(5),
                socket_close_delay: embassy_time::Duration::from_millis(2),
            },
        );
        assert_eq!(client_small_custom.options.max_retries, 2);
    }

    #[test]
    fn test_parse_http_response_binary_body() {
        // Simulate a PNG-like response with invalid UTF-8 in the body
        let header = b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 8\r\n\r\n";
        let binary_body: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]; // PNG magic bytes
        let mut data = [0u8; 256];
        data[..header.len()].copy_from_slice(header);
        data[header.len()..header.len() + binary_body.len()].copy_from_slice(&binary_body);
        let data = &data[..header.len() + binary_body.len()];

        let response = DefaultHttpClient::parse_http_response_zero_copy(data)
            .expect("should parse binary response");

        assert_eq!(response.status_code, StatusCode::Ok);
        assert!(matches!(response.body, ResponseBody::Binary(b) if b == binary_body));
    }

    #[test]
    fn test_parse_http_response_text_body() {
        let data = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nhello";

        let response = DefaultHttpClient::parse_http_response_zero_copy(data)
            .expect("should parse text response");

        assert_eq!(response.status_code, StatusCode::Ok);
        assert!(matches!(response.body, ResponseBody::Text("hello")));
    }

    #[test]
    fn test_is_response_complete_chunked() {
        let incomplete = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n";
        assert!(!DefaultHttpClient::is_response_complete(incomplete));

        let complete =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
        assert!(DefaultHttpClient::is_response_complete(complete));
    }

    #[test]
    fn test_dechunk_single_chunk() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: text/plain\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
        let mut buf = [0u8; 256];
        buf[..raw.len()].copy_from_slice(raw);

        let new_len =
            DefaultHttpClient::dechunk(&mut buf, raw.len()).expect("should decode chunked");

        let response = DefaultHttpClient::parse_http_response_zero_copy(&buf[..new_len])
            .expect("should parse dechunked response");

        assert_eq!(response.status_code, StatusCode::Ok);
        assert_eq!(response.body.as_str(), Some("hello"));
    }

    #[test]
    fn test_dechunk_multiple_chunks() {
        // Mimics the weather API response from issue #29
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: application/json\r\n\r\nb\r\n{\"temp\":23}\r\n0\r\n\r\n";
        let mut buf = [0u8; 256];
        buf[..raw.len()].copy_from_slice(raw);

        let new_len =
            DefaultHttpClient::dechunk(&mut buf, raw.len()).expect("should decode chunked");

        let response = DefaultHttpClient::parse_http_response_zero_copy(&buf[..new_len])
            .expect("should parse dechunked response");

        assert_eq!(response.body.as_str(), Some("{\"temp\":23}"));
    }

    #[test]
    fn test_dechunk_noop_when_not_chunked() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
        let mut buf = [0u8; 128];
        buf[..raw.len()].copy_from_slice(raw);

        let new_len = DefaultHttpClient::dechunk(&mut buf, raw.len()).expect("should pass through");
        assert_eq!(new_len, raw.len());
    }
}
