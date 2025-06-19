#![no_std]
//! # Nanofish
//!
//! A lightweight, `no_std` HTTP client for embedded systems built on top of Embassy networking.
//!
//! Nanofish provides a simple HTTP client implementation that works on constrained environments
//! with no heap allocation, making it suitable for microcontrollers and other embedded systems.
//! It supports common HTTP methods (GET, POST, PUT, DELETE, etc.) and provides a clean API
//! for making HTTP requests.
//!
//! ## Features
//!
//! - Full `no_std` compatibility with no heap allocations
//! - Built on Embassy for async networking
//! - Support for all standard HTTP methods
//! - Automatic handling of common headers
//! - DNS resolution
//! - Timeout handling and retries
//!
//! ## Example
//!
//! ```no_run
//! use nanofish::{HttpClient, HttpHeader};
//! use embassy_net::Stack;
//!
//! async fn example(stack: &Stack<'_>) -> Result<(), nanofish::Error> {
//!     // Create an HTTP client with a network stack
//!     let client = HttpClient::new(stack);
//!     // Define custom headers (optional)
//!     let headers = [
//!         HttpHeader { name: "User-Agent", value: "Nanofish/0.1.0" },
//!     ];
//!     // Make a GET request
//!     let response = client.get("http://example.com/api/status", &headers).await?;
//!     // Check the response
//!     if response.status_code == 200 {
//!         // Process the response body
//!         let body = response.body;
//!     }
//!     Ok(())
//! }
//! ```

use core::fmt::Write;
use defmt::{debug, error};
use embassy_net::{
    Stack,
    dns::{self, DnsSocket},
    tcp::TcpSocket,
};
use embassy_time::{Duration, Timer};
use embedded_io_async::Write as EmbeddedWrite;
use heapless::{String, Vec};

/// Maximum number of retries for read operations
const MAX_RETRIES: usize = 5;
/// Maximum size for HTTP request buffers
const REQUEST_SIZE: usize = 1024;
/// Size of the TCP transmission buffer
const TRANSMIT_BUFFER_SIZE: usize = 4096;
/// Size of the TCP receive buffer
const RECEIVE_BUFFER_SIZE: usize = 4096;
/// Size of the HTTP response buffer
const RESPONSE_BUFFER_SIZE: usize = 4096;
/// Timeout duration for socket operations
const SOCKET_TIMEOUT: Duration = Duration::from_secs(10);
/// Delay between retry attempts
const RETRY_DELAY: Duration = Duration::from_millis(200);
/// Delay after closing a socket before proceeding
const SOCKET_CLOSE_DELAY: Duration = Duration::from_millis(100);
/// Maximum size for HTTP response body
const RESPONSE_SIZE: usize = 2048;

/// Errors that can occur during HTTP operations
///
/// This enum represents all possible errors that can be returned by the HTTP client
/// during various stages of request processing, from URL parsing to connection
/// establishment and response handling.
#[derive(Debug)]
pub enum Error {
    /// The provided URL was invalid or malformed
    InvalidUrl,
    /// DNS resolution failed
    DnsError(dns::Error),
    /// No IP addresses were returned by DNS resolution
    IpAddressEmpty,
    /// Failed to establish a TCP connection
    ConnectionError(embassy_net::tcp::ConnectError),
    /// TCP communication error
    TcpError(embassy_net::tcp::Error),
    /// No response was received from the server
    NoResponse,
    /// The server's response could not be parsed
    InvalidResponse(&'static str),
}

impl defmt::Format for Error {
    fn format(&self, fmt: defmt::Formatter) {
        defmt::write!(fmt, "{:?}", self)
    }
}

impl From<dns::Error> for Error {
    fn from(err: dns::Error) -> Self {
        Error::DnsError(err)
    }
}

impl From<embassy_net::tcp::ConnectError> for Error {
    fn from(err: embassy_net::tcp::ConnectError) -> Self {
        Error::ConnectionError(err)
    }
}

impl From<embassy_net::tcp::Error> for Error {
    fn from(err: embassy_net::tcp::Error) -> Self {
        Error::TcpError(err)
    }
}

/// HTTP Methods supported by the client
///
/// This enum represents the standard HTTP methods that can be used
/// when making requests with the `HttpClient`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HttpMethod {
    /// The GET method requests a representation of the specified resource.
    /// Requests using GET should only retrieve data.
    GET,
    /// The POST method is used to submit an entity to the specified resource,
    /// often causing a change in state or side effects on the server.
    POST,
    /// The PUT method replaces all current representations of the target
    /// resource with the request payload.
    PUT,
    /// The DELETE method deletes the specified resource.
    DELETE,
    /// The PATCH method is used to apply partial modifications to a resource.
    PATCH,
    /// The CONNECT method establishes a tunnel to the server identified by the target resource.
    CONNECT,
    /// The OPTIONS method is used to describe the communication options for the target resource.
    OPTIONS,
    /// The TRACE method performs a message loop-back test along the path to the target resource.
    TRACE,
    /// The HEAD method asks for a response identical to that of a GET request,
    /// but without the response body.
    HEAD,
}

macro_rules! try_push {
    ($expr:expr) => {
        if $expr.is_err() {
            return Err(Error::InvalidResponse("Request buffer overflow"));
        }
    };
}

impl HttpMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            HttpMethod::GET => "GET",
            HttpMethod::POST => "POST",
            HttpMethod::PUT => "PUT",
            HttpMethod::DELETE => "DELETE",
            HttpMethod::PATCH => "PATCH",
            HttpMethod::CONNECT => "CONNECT",
            HttpMethod::OPTIONS => "OPTIONS",
            HttpMethod::TRACE => "TRACE",
            HttpMethod::HEAD => "HEAD",
        }
    }
}

/// HTTP Header struct for representing a single header
///
/// This struct represents a single HTTP header with a name and value.
/// Headers are used to pass additional information about the request or response.
#[derive(Clone, Debug)]
pub struct HttpHeader<'a> {
    /// The name of the header (e.g., "Content-Type", "Authorization")
    pub name: &'a str,
    /// The value of the header (e.g., "application/json", "Bearer token123")
    pub value: &'a str,
}

/// HTTP Response struct with status code, headers and body
///
/// This struct represents the response received from an HTTP server.
/// It contains the status code, headers, and the response body.
pub struct HttpClientResponse {
    /// The HTTP status code (e.g., 200 for OK, 404 for Not Found)
    pub status_code: u16,
    /// A collection of response headers
    pub headers: Vec<HttpHeader<'static>, 16>,
    /// The response body as a string with a maximum capacity of 2048 bytes
    pub body: String<2048>,
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
}

impl<'a> HttpClient<'a> {
    /// Create a new HTTP client
    ///
    /// Creates a new HTTP client with the provided Embassy network stack.
    ///
    /// # Arguments
    ///
    /// * `stack` - A reference to an Embassy network stack
    ///
    /// # Returns
    ///
    /// A new instance of `HttpClient`
    pub fn new(stack: &'a Stack<'a>) -> Self {
        Self { stack }
    }

    /// Make an HTTP request
    ///
    /// This is the core method for making HTTP requests. It handles all HTTP methods
    /// and manages the entire request flow, from DNS resolution to parsing the response.
    ///
    /// # Arguments
    ///
    /// * `method` - The HTTP method to use (GET, POST, etc.)
    /// * `endpoint` - The URL to request (e.g., "http://example.com/api")
    /// * `headers` - A slice of HTTP headers to include in the request
    /// * `body` - Optional request body data (required for POST/PUT requests)
    ///
    /// # Returns
    ///
    /// * `Ok(HttpClientResponse)` - Successful response with status code, headers, and body
    /// * `Err(Error)` - Error occurred during the request process
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use nanofish::{HttpClient, HttpHeader, HttpMethod};
    /// use embassy_net::Stack;
    ///
    /// async fn example(client: &HttpClient<'_>) -> Result<(), nanofish::Error> {
    ///     // Making a simple GET request
    ///     let response = client.request(HttpMethod::GET, "http://example.com", &[], None).await?;
    ///
    ///     // Making a POST request with JSON data
    ///     let json = b"{\"key\": \"value\"}";
    ///     let headers = [HttpHeader { name: "Content-Type", value: "application/json" }];
    ///     let response = client.request(HttpMethod::POST, "http://api.example.com/data", &headers, Some(json)).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn request(
        &self,
        method: HttpMethod,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        body: Option<&[u8]>,
    ) -> Result<HttpClientResponse, Error> {
        // Parse scheme, host, and port
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
        debug!(
            "Connecting to host: {}, port: {}, path: {} (scheme: {})",
            host, port, path, scheme
        );

        let mut rx_buffer = [0; RECEIVE_BUFFER_SIZE];
        let mut tx_buffer = [0; TRANSMIT_BUFFER_SIZE];
        let mut socket = TcpSocket::new(*self.stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(SOCKET_TIMEOUT));

        let dns_socket = DnsSocket::new(*self.stack);

        debug!("Resolving DNS for host: {}", host);
        let ip_addresses = dns_socket
            .query(host, embassy_net::dns::DnsQueryType::A)
            .await?;

        if ip_addresses.is_empty() {
            return Err(Error::IpAddressEmpty);
        }

        let ip_addr = ip_addresses[0];
        debug!("Resolved {} to {:?}", host, ip_addr);
        let remote_endpoint = (ip_addr, port);

        // For HTTPS, wrap the socket in a TLS stream here using a TLS library
        // e.g., using embedded-tls or rustls (not shown here)
        // let mut tls_stream = TlsStream::new(socket, ...);
        // Use tls_stream for read/write if scheme == "https"

        socket
            .connect(remote_endpoint)
            .await
            .map_err(|e: embassy_net::tcp::ConnectError| {
                socket.abort();
                Error::from(e)
            })?;

        let mut http_request = String::<REQUEST_SIZE>::new();

        try_push!(http_request.push_str(method.as_str()));
        try_push!(http_request.push_str(" "));
        try_push!(http_request.push_str(path));
        try_push!(http_request.push_str(" HTTP/1.1\r\n"));
        try_push!(http_request.push_str("Host: "));
        try_push!(http_request.push_str(host));
        try_push!(http_request.push_str("\r\n"));

        let mut content_type_present = false;
        let mut content_length_present = false;

        for header in headers {
            try_push!(http_request.push_str(header.name));
            try_push!(http_request.push_str(": "));
            try_push!(http_request.push_str(header.value));
            try_push!(http_request.push_str("\r\n"));

            if header.name.eq_ignore_ascii_case("Content-Type") {
                content_type_present = true;
            } else if header.name.eq_ignore_ascii_case("Content-Length") {
                content_length_present = true;
            }
        }

        if !content_type_present
            && body.is_some()
            && (method == HttpMethod::POST || method == HttpMethod::PUT)
        {
            try_push!(http_request.push_str("Content-Type: application/json\r\n"));
        }

        if !content_length_present && body.is_some() {
            try_push!(http_request.push_str("Content-Length: "));
            let mut len_str = String::<8>::new();
            if write!(&mut len_str, "{}", body.unwrap().len()).is_err() {
                return Err(Error::InvalidResponse("Failed to write content length"));
            }
            try_push!(http_request.push_str(&len_str));
            try_push!(http_request.push_str("\r\n"));
        }

        try_push!(http_request.push_str("Connection: close\r\n"));
        try_push!(http_request.push_str("\r\n"));

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
        let mut retries = MAX_RETRIES;

        while total_read < response_buffer.len() && retries > 0 {
            match socket.read(&mut response_buffer[total_read..]).await {
                Ok(0) => {
                    debug!("Connection closed by server");
                    break;
                }
                Ok(n) => {
                    total_read += n;
                    debug!("Read {} bytes, total: {}", n, total_read);

                    let response_str =
                        core::str::from_utf8(&response_buffer[..total_read]).unwrap_or_default();
                    if response_str.contains("\r\n\r\n") {
                        debug!("Complete HTTP response received");

                        if let Some(content_length_pos) = response_str.find("Content-Length:") {
                            let content_length_end = response_str[content_length_pos..]
                                .find("\r\n")
                                .unwrap_or_default()
                                + content_length_pos;
                            let content_length_str =
                                &response_str[content_length_pos + 15..content_length_end].trim();

                            if let Ok(content_length) = content_length_str.parse::<usize>() {
                                let headers_end =
                                    response_str.find("\r\n\r\n").unwrap_or_default() + 4;
                                let body_received = total_read.saturating_sub(headers_end);

                                if body_received >= content_length {
                                    debug!("Received full body of size {}", content_length);
                                    break;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("Socket read error: {:?}", e);
                    retries -= 1;
                    if retries > 0 {
                        Timer::after(RETRY_DELAY).await;
                    }
                }
            }
        }

        socket.close();

        Timer::after(SOCKET_CLOSE_DELAY).await;

        if total_read == 0 {
            return Err(Error::NoResponse);
        }

        debug!("Received HTTP response, {} bytes", total_read);

        // Remove unwrap_or_default on from_utf8
        let response_str = core::str::from_utf8(&response_buffer[..total_read])
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
            .find("\r\r\n\r\n")
            .ok_or(Error::InvalidResponse("Invalid HTTP response format"))?
            + 4;

        let headers_section = &response_str[status_line_end + 2..headers_end - 4];
        let mut headers = Vec::<HttpHeader<'static>, 16>::new();
        for header_line in headers_section.split("\r\n") {
            if let Some(colon_pos) = header_line.find(':') {
                let name = header_line[..colon_pos].trim();

                let name_static = match name {
                    "Content-Type" => "Content-Type",
                    "Content-Length" => "Content-Length",
                    "Connection" => "Connection",
                    "Server" => "Server",
                    "Date" => "Date",
                    _ => "X-Header",
                };

                if headers
                    .push(HttpHeader {
                        name: name_static,
                        value: "",
                    })
                    .is_err()
                {
                    debug!("Too many headers, skipping some");
                    break;
                }
            }
        }

        let body_text = if headers_end < response_str.len() {
            &response_str[headers_end..]
        } else {
            ""
        };

        let mut body = String::<RESPONSE_SIZE>::new();
        let _ = body.push_str(body_text); // ignore error, body will be truncated if too large

        Ok(HttpClientResponse {
            status_code,
            headers,
            body,
        })
    }

    /// Convenience method for making a PATCH request
    ///
    /// # Arguments
    /// * `endpoint` - The URL to request (e.g., "http://example.com/api")
    /// * `headers` - A slice of HTTP headers to include in the request
    /// * `body` - The request body data
    ///
    /// # Returns
    /// * `Ok(HttpClientResponse)` - Successful response
    /// * `Err(Error)` - Error occurred during the request process
    pub async fn patch(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        body: &[u8],
    ) -> Result<HttpClientResponse, Error> {
        self.request(HttpMethod::PATCH, endpoint, headers, Some(body))
            .await
    }

    /// Convenience method for making a HEAD request
    ///
    /// # Arguments
    /// * `endpoint` - The URL to request (e.g., "http://example.com/api")
    /// * `headers` - A slice of HTTP headers to include in the request
    ///
    /// # Returns
    /// * `Ok(HttpClientResponse)` - Successful response
    /// * `Err(Error)` - Error occurred during the request process
    pub async fn head(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
    ) -> Result<HttpClientResponse, Error> {
        self.request(HttpMethod::HEAD, endpoint, headers, None)
            .await
    }

    /// Convenience method for making an OPTIONS request
    ///
    /// # Arguments
    /// * `endpoint` - The URL to request (e.g., "http://example.com/api")
    /// * `headers` - A slice of HTTP headers to include in the request
    ///
    /// # Returns
    /// * `Ok(HttpClientResponse)` - Successful response
    /// * `Err(Error)` - Error occurred during the request process
    pub async fn options(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
    ) -> Result<HttpClientResponse, Error> {
        self.request(HttpMethod::OPTIONS, endpoint, headers, None)
            .await
    }

    /// Convenience method for making a TRACE request
    ///
    /// # Arguments
    /// * `endpoint` - The URL to request (e.g., "http://example.com/api")
    /// * `headers` - A slice of HTTP headers to include in the request
    ///
    /// # Returns
    /// * `Ok(HttpClientResponse)` - Successful response
    /// * `Err(Error)` - Error occurred during the request process
    pub async fn trace(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
    ) -> Result<HttpClientResponse, Error> {
        self.request(HttpMethod::TRACE, endpoint, headers, None)
            .await
    }

    /// Convenience method for making a CONNECT request
    ///
    /// # Arguments
    /// * `endpoint` - The URL to request (e.g., "http://example.com/api")
    /// * `headers` - A slice of HTTP headers to include in the request
    ///
    /// # Returns
    /// * `Ok(HttpClientResponse)` - Successful response
    /// * `Err(Error)` - Error occurred during the request process
    pub async fn connect(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
    ) -> Result<HttpClientResponse, Error> {
        self.request(HttpMethod::CONNECT, endpoint, headers, None)
            .await
    }

    /// Convenience method for making a GET request
    ///
    /// # Arguments
    /// * `endpoint` - The URL to request (e.g., "http://example.com/api")
    /// * `headers` - A slice of HTTP headers to include in the request
    ///
    /// # Returns
    /// * `Ok(HttpClientResponse)` - Successful response
    /// * `Err(Error)` - Error occurred during the request process
    pub async fn get(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
    ) -> Result<HttpClientResponse, Error> {
        self.request(HttpMethod::GET, endpoint, headers, None).await
    }

    /// Convenience method for making a POST request
    ///
    /// # Arguments
    /// * `endpoint` - The URL to request (e.g., "http://example.com/api")
    /// * `headers` - A slice of HTTP headers to include in the request
    /// * `body` - The request body data
    ///
    /// # Returns
    /// * `Ok(HttpClientResponse)` - Successful response
    /// * `Err(Error)` - Error occurred during the request process
    pub async fn post(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        body: &[u8],
    ) -> Result<HttpClientResponse, Error> {
        self.request(HttpMethod::POST, endpoint, headers, Some(body))
            .await
    }

    /// Convenience method for making a PUT request
    ///
    /// # Arguments
    /// * `endpoint` - The URL to request (e.g., "http://example.com/api")
    /// * `headers` - A slice of HTTP headers to include in the request
    /// * `body` - The request body data
    ///
    /// # Returns
    /// * `Ok(HttpClientResponse)` - Successful response
    /// * `Err(Error)` - Error occurred during the request process
    pub async fn put(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        body: &[u8],
    ) -> Result<HttpClientResponse, Error> {
        self.request(HttpMethod::PUT, endpoint, headers, Some(body))
            .await
    }

    /// Convenience method for making a DELETE request
    ///
    /// # Arguments
    /// * `endpoint` - The URL to request (e.g., "http://example.com/api")
    /// * `headers` - A slice of HTTP headers to include in the request
    ///
    /// # Returns
    /// * `Ok(HttpClientResponse)` - Successful response
    /// * `Err(Error)` - Error occurred during the request process
    pub async fn delete(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
    ) -> Result<HttpClientResponse, Error> {
        self.request(HttpMethod::DELETE, endpoint, headers, None)
            .await
    }
}
