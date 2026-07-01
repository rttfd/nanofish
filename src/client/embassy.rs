use super::io::{HttpClient as BaseHttpClient, HttpClientRequest, parse_endpoint};
use crate::{
    error::Error, header::HttpHeader, method::HttpMethod, options::HttpClientOptions,
    response::HttpResponse,
};
use embassy_net::{
    Stack,
    dns::{self, DnsSocket},
    tcp::TcpSocket,
};
#[cfg(feature = "tls")]
use embassy_time::Instant;
use embassy_time::Timer;
#[cfg(feature = "tls")]
use embedded_tls::{Aes128GcmSha256, TlsConfig, TlsConnection, TlsContext, UnsecureProvider};
#[cfg(feature = "tls")]
use rand_core::{CryptoRng, RngCore};

const REQUEST_SIZE: usize = 1024;
const SMALL_BUFFER_SIZE: usize = 1024;
const MEDIUM_BUFFER_SIZE: usize = 4096;

/// Type alias for `EmbassyHttpClient` with default buffer sizes
pub type DefaultEmbassyHttpClient<'a> = EmbassyHttpClient<
    'a,
    MEDIUM_BUFFER_SIZE, // TCP_RX: 4KB
    MEDIUM_BUFFER_SIZE, // TCP_TX: 4KB
    MEDIUM_BUFFER_SIZE, // TLS_READ: 4KB
    MEDIUM_BUFFER_SIZE, // TLS_WRITE: 4KB
    REQUEST_SIZE,       // RQ: 1KB
>;

/// Type alias for `EmbassyHttpClient` with small buffer sizes for memory-constrained environments
pub type SmallEmbassyHttpClient<'a> = EmbassyHttpClient<
    'a,
    SMALL_BUFFER_SIZE, // TCP_RX: 1KB
    SMALL_BUFFER_SIZE, // TCP_TX: 1KB
    SMALL_BUFFER_SIZE, // TLS_READ: 1KB
    SMALL_BUFFER_SIZE, // TLS_WRITE: 1KB
    REQUEST_SIZE,      // RQ: 1KB
>;

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
pub struct EmbassyHttpClient<
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
> EmbassyHttpClient<'a, TCP_RX, TCP_TX, TLS_READ, TLS_WRITE, RQ>
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
    pub const fn with_options(stack: &'a Stack<'a>, options: HttpClientOptions) -> Self {
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
    /// use nanofish::{DefaultEmbassyHttpClient, HttpHeader, HttpMethod, ResponseBody};
    /// use embassy_net::Stack;
    ///
    /// async fn example(stack: &Stack<'_>) -> Result<(), nanofish::Error> {
    ///     let client = DefaultEmbassyHttpClient::new(stack);
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
    #[expect(clippy::future_not_send)]
    pub async fn request<'b>(
        &self,
        method: HttpMethod,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        body: Option<&[u8]>,
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error> {
        let endpoint = parse_endpoint(endpoint)?;

        match endpoint.scheme {
            #[cfg(feature = "tls")]
            "https" => {
                self.make_https_request(
                    method,
                    (endpoint.host, endpoint.port),
                    endpoint.path,
                    headers,
                    body,
                    response_buffer,
                )
                .await
            }
            #[cfg(not(feature = "tls"))]
            "https" => Err(Error::UnsupportedScheme("https (TLS support not enabled)")),
            "http" => {
                self.make_http_request(
                    method,
                    (endpoint.host, endpoint.port),
                    endpoint.path,
                    headers,
                    body,
                    response_buffer,
                )
                .await
            }
            _ => Err(Error::UnsupportedScheme("unknown")),
        }
    }

    /// Resolve a hostname to an IP address, trying IPv4 (A) first then IPv6 (AAAA).
    #[expect(clippy::future_not_send)]
    async fn resolve_host(stack: Stack<'_>, host: &str) -> Result<embassy_net::IpAddress, Error> {
        let dns_socket = DnsSocket::new(stack);

        // Try A (IPv4) first — most common on embedded networks
        if let Ok(addrs) = dns_socket.query(host, dns::DnsQueryType::A).await
            && let Some(&addr) = addrs.first()
        {
            return Ok(addr);
        }

        // Fall back to AAAA (IPv6)
        let addrs = dns_socket.query(host, dns::DnsQueryType::Aaaa).await?;
        addrs.first().copied().ok_or(Error::IpAddressEmpty)
    }

    /// Make HTTPS request over TLS with zero-copy response handling
    #[cfg(feature = "tls")]
    #[expect(clippy::future_not_send)]
    async fn make_https_request<'b>(
        &self,
        method: HttpMethod,
        host_port: (&str, u16),
        path: &str,
        headers: &[HttpHeader<'_>],
        body: Option<&[u8]>,
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error> {
        let (host, port) = host_port;
        let mut rx_buffer = [0; TCP_RX];
        let mut tx_buffer = [0; TCP_TX];
        let mut socket = TcpSocket::new(*self.stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(embassy_time::Duration::from_millis(
            self.options.socket_timeout.as_millis(),
        )));

        let ip_addr = Self::resolve_host(*self.stack, host).await?;
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
        let timeseed_bytes = timeseed();
        let seed = u32::from_be_bytes([
            timeseed_bytes[0],
            timeseed_bytes[1],
            timeseed_bytes[2],
            timeseed_bytes[3],
        ]);
        let rng = XorShift32Rng::new(seed);

        tls.open(TlsContext::new(
            &tls_config,
            UnsecureProvider::new::<Aes128GcmSha256>(rng),
        ))
        .await?;

        let client = BaseHttpClient::<RQ>::with_options(self.options);
        let result = client
            .request(
                &mut tls,
                HttpClientRequest {
                    method,
                    host,
                    path,
                    headers,
                    body,
                },
                response_buffer,
            )
            .await;

        if let Err((_, e)) = tls.close().await {
            debug!("Error closing TLS connection: {:?}", Error::from(e));
        }

        Timer::after(embassy_time::Duration::from_millis(
            self.options.socket_close_delay.as_millis(),
        ))
        .await;

        result
    }

    /// Make HTTP request with zero-copy response handling
    #[expect(clippy::future_not_send)]
    async fn make_http_request<'b>(
        &self,
        method: HttpMethod,
        host_port: (&str, u16),
        path: &str,
        headers: &[HttpHeader<'_>],
        body: Option<&[u8]>,
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error> {
        let (host, port) = host_port;
        let mut rx_buffer = [0; TCP_RX];
        let mut tx_buffer = [0; TCP_TX];
        let mut socket = TcpSocket::new(*self.stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(embassy_time::Duration::from_millis(
            self.options.socket_timeout.as_millis(),
        )));

        let ip_addr = Self::resolve_host(*self.stack, host).await?;
        let remote_endpoint = (ip_addr, port);

        socket
            .connect(remote_endpoint)
            .await
            .map_err(|e: embassy_net::tcp::ConnectError| {
                socket.abort();
                Error::from(e)
            })?;

        let client = BaseHttpClient::<RQ>::with_options(self.options);
        let result = client
            .request(
                &mut socket,
                HttpClientRequest {
                    method,
                    host,
                    path,
                    headers,
                    body,
                },
                response_buffer,
            )
            .await;

        socket.close();
        Timer::after(embassy_time::Duration::from_millis(
            self.options.socket_close_delay.as_millis(),
        ))
        .await;

        result
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
    /// Returns the same errors as [`EmbassyHttpClient::request`].
    #[expect(clippy::future_not_send)]
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
    /// Returns the same errors as [`EmbassyHttpClient::request`].
    #[expect(clippy::future_not_send)]
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
    /// Returns the same errors as [`EmbassyHttpClient::request`].
    #[expect(clippy::future_not_send)]
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
    /// Returns the same errors as [`EmbassyHttpClient::request`].
    #[expect(clippy::future_not_send)]
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
    /// Returns the same errors as [`EmbassyHttpClient::request`].
    #[expect(clippy::future_not_send)]
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
    /// Returns the same errors as [`EmbassyHttpClient::request`].
    #[expect(clippy::future_not_send)]
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
    /// Returns the same errors as [`EmbassyHttpClient::request`].
    #[expect(clippy::future_not_send)]
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
    /// Returns the same errors as [`EmbassyHttpClient::request`].
    #[expect(clippy::future_not_send)]
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
    /// Returns the same errors as [`EmbassyHttpClient::request`].
    #[expect(clippy::future_not_send)]
    pub async fn delete<'b>(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error> {
        self.request(HttpMethod::DELETE, endpoint, headers, None, response_buffer)
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

/// Simple `XORShift32` PRNG for TLS seeding.
/// Good enough for embedded TLS where the seed itself is time-based.
#[cfg(feature = "tls")]
struct XorShift32Rng(u32);

#[cfg(feature = "tls")]
impl XorShift32Rng {
    const fn new(seed: u32) -> Self {
        Self(seed)
    }
}

#[cfg(feature = "tls")]
impl RngCore for XorShift32Rng {
    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    fn next_u64(&mut self) -> u64 {
        (u64::from(self.next_u32()) << 32) | u64::from(self.next_u32())
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        for chunk in dest.chunks_mut(4) {
            let val = self.next_u32();
            chunk.copy_from_slice(&val.to_le_bytes()[..chunk.len()]);
        }
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

#[cfg(feature = "tls")]
impl CryptoRng for XorShift32Rng {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ResponseBody, StatusCode, codec};
    use embassy_net::Stack;

    #[test]
    fn test_is_response_complete_no_content_length() {
        // Without Content-Length or chunked, response is never "complete" —
        // the read loop must rely on connection close (Ok(0))
        let data = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\n";
        assert!(!codec::complete(data));
    }

    #[test]
    fn test_is_response_complete_content_length_zero() {
        // Content-Length: 0 means empty body — complete once headers end
        let data = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
        assert!(codec::complete(data));
    }

    #[test]
    fn test_is_response_complete_with_content_length() {
        let data = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
        assert!(codec::complete(data));
    }

    #[test]
    fn test_is_response_complete_incomplete() {
        let data = b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nshort";
        assert!(!codec::complete(data));
    }

    #[test]
    fn test_new_and_with_options() {
        // This test only checks that the options are set correctly, not that the stack is valid.
        // Use a raw pointer to avoid UB and static mut issues. This is safe for type-checking only.
        let fake_stack: *const Stack = core::ptr::NonNull::dangling().as_ptr();
        let client = DefaultEmbassyHttpClient::new(unsafe { &*fake_stack });
        let opts = HttpClientOptions {
            max_retries: 1,
            socket_timeout: crate::TimeoutDuration::from_secs(1),
            retry_delay: crate::TimeoutDuration::from_millis(1),
            socket_close_delay: crate::TimeoutDuration::from_millis(1),
        };
        let client2 = DefaultEmbassyHttpClient::with_options(unsafe { &*fake_stack }, opts);
        assert_eq!(client.options.max_retries, 5);
        assert_eq!(client2.options.max_retries, 1);
    }

    #[test]
    fn test_default_http_client_constructors() {
        let fake_stack: *const Stack = core::ptr::NonNull::dangling().as_ptr();
        let client_default = DefaultEmbassyHttpClient::new(unsafe { &*fake_stack });
        assert_eq!(client_default.options.max_retries, 5);

        let client_custom = DefaultEmbassyHttpClient::with_options(
            unsafe { &*fake_stack },
            HttpClientOptions {
                max_retries: 3,
                socket_timeout: crate::TimeoutDuration::from_secs(2),
                retry_delay: crate::TimeoutDuration::from_millis(10),
                socket_close_delay: crate::TimeoutDuration::from_millis(5),
            },
        );
        assert_eq!(client_custom.options.max_retries, 3);
    }

    #[test]
    fn test_small_http_client_constructors() {
        let fake_stack: *const Stack = core::ptr::NonNull::dangling().as_ptr();
        let client_small = SmallEmbassyHttpClient::new(unsafe { &*fake_stack });
        assert_eq!(client_small.options.max_retries, 5);

        let client_small_custom = SmallEmbassyHttpClient::with_options(
            unsafe { &*fake_stack },
            HttpClientOptions {
                max_retries: 2,
                socket_timeout: crate::TimeoutDuration::from_secs(1),
                retry_delay: crate::TimeoutDuration::from_millis(5),
                socket_close_delay: crate::TimeoutDuration::from_millis(2),
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

        let response = codec::parse_resp(data).expect("should parse binary response");

        assert_eq!(response.status_code, StatusCode::Ok);
        assert!(matches!(response.body, ResponseBody::Binary(b) if b == binary_body));
    }

    #[test]
    fn test_parse_http_response_text_body() {
        let data = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nhello";

        let response = codec::parse_resp(data).expect("should parse text response");

        assert_eq!(response.status_code, StatusCode::Ok);
        assert!(matches!(response.body, ResponseBody::Text("hello")));
    }

    #[test]
    fn test_is_response_complete_chunked() {
        let incomplete = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n";
        assert!(!codec::complete(incomplete));

        let complete =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
        assert!(codec::complete(complete));
    }

    #[test]
    fn test_dechunk_single_chunk() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: text/plain\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
        let mut buf = [0u8; 256];
        buf[..raw.len()].copy_from_slice(raw);

        let new_len = codec::dechunk(&mut buf, raw.len()).expect("should decode chunked");

        let response = codec::parse_resp(&buf[..new_len]).expect("should parse dechunked response");

        assert_eq!(response.status_code, StatusCode::Ok);
        assert_eq!(response.body.as_str(), Some("hello"));
    }

    #[test]
    fn test_dechunk_multiple_chunks() {
        // Mimics the weather API response from issue #29
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: application/json\r\n\r\nb\r\n{\"temp\":23}\r\n0\r\n\r\n";
        let mut buf = [0u8; 256];
        buf[..raw.len()].copy_from_slice(raw);

        let new_len = codec::dechunk(&mut buf, raw.len()).expect("should decode chunked");

        let response = codec::parse_resp(&buf[..new_len]).expect("should parse dechunked response");

        assert_eq!(response.body.as_str(), Some("{\"temp\":23}"));
    }

    #[test]
    fn test_dechunk_noop_when_not_chunked() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
        let mut buf = [0u8; 128];
        buf[..raw.len()].copy_from_slice(raw);

        let new_len = codec::dechunk(&mut buf, raw.len()).expect("should pass through");
        assert_eq!(new_len, raw.len());
    }
}
