use core::future::Future;
use embassy_net::{
    Stack,
    dns::{self, DnsSocket},
    tcp::TcpSocket,
};
use embassy_time::{Timer, with_timeout};
use heapless::String;
use nanofish::client::builder::RequestBuilder;
use nanofish::client::io::{
    HttpClient as BaseHttpClient, HttpClientRequest, HttpEndpoint, parse_endpoint,
};
use nanofish::client::redirect::follow;
use nanofish::client::stream::{BodySink, StreamedResponse};
use nanofish::{
    codec, error::Error, header::HttpHeader, method::HttpMethod, options::HttpClientOptions,
    response::HttpResponse,
};

// HTTPS support is one of two files, so this one compiles the same either way.
#[cfg(not(feature = "tls"))]
mod no_tls;
#[cfg(feature = "tls")]
mod tls;

const REQUEST_SIZE: usize = 1024;
const SMALL_BUFFER_SIZE: usize = 1024;
const MEDIUM_BUFFER_SIZE: usize = 4096;
/// Room for a 253-byte hostname plus `:` and a five-digit port.
const HOST_HEADER_SIZE: usize = 259;

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
/// # Security
///
/// With the `tls` feature, HTTPS connections **do not verify the server
/// certificate** (`embedded-tls` `UnsecureProvider`): the server's identity is
/// not authenticated, so connections are vulnerable to man-in-the-middle
/// attacks. Handshake randomness is derived from the system tick counter, not
/// a cryptographic RNG. Only use HTTPS on trusted networks, or terminate TLS
/// elsewhere (for example on a reverse proxy).
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
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
    pub async fn request<'b>(
        &self,
        method: HttpMethod,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        body: Option<&[u8]>,
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error> {
        let attempt =
            self.request_following_redirects(method, endpoint, headers, body, response_buffer);

        match self.options.total_timeout {
            Some(limit) => with_timeout(
                embassy_time::Duration::from_millis(limit.as_millis()),
                attempt,
            )
            .await
            .unwrap_or(Err(Error::Timeout)),
            None => attempt.await,
        }
    }

    /// Send the request, then follow up to [`HttpClientOptions::max_redirects`]
    /// redirects.
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
    async fn request_following_redirects<'b>(
        &self,
        method: HttpMethod,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        body: Option<&[u8]>,
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error> {
        follow(
            self.options.max_redirects,
            (method, endpoint, headers, body),
            response_buffer,
            async |method, endpoint, headers, body, buffer| {
                self.request_once(method, endpoint, headers, body, buffer)
                    .await
                    .map(|(_, used)| used)
            },
        )
        .await
    }

    /// Send one request without following redirects.
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
    async fn request_once<'b>(
        &self,
        method: HttpMethod,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        body: Option<&[u8]>,
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error> {
        let endpoint = parse_endpoint(endpoint)?;

        match endpoint.scheme {
            "https" => {
                self.make_https_request(method, &endpoint, headers, body, response_buffer)
                    .await
            }
            "http" => {
                self.make_http_request(method, &endpoint, headers, body, response_buffer)
                    .await
            }
            _ => Err(Error::UnsupportedScheme("unknown")),
        }
    }

    /// Connect to `endpoint` over plain TCP, run `operation` on the socket,
    /// and close it.
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
    async fn with_http_connection<R>(
        &self,
        endpoint: &HttpEndpoint<'_>,
        operation: impl AsyncFnOnce(&mut TcpSocket<'_>) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let (host, port) = (endpoint.host, endpoint.port);
        let mut rx_buffer = [0; TCP_RX];
        let mut tx_buffer = [0; TCP_TX];
        let mut socket = TcpSocket::new(*self.stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(embassy_time::Duration::from_millis(
            self.options.socket_timeout.as_millis(),
        )));

        let ip_addr = resolve_host(*self.stack, host).await?;
        let remote_endpoint = (ip_addr, port);

        socket
            .connect(remote_endpoint)
            .await
            .map_err(|e: embassy_net::tcp::ConnectError| {
                socket.abort();
                Error::from(e)
            })?;

        let result = operation(&mut socket).await;

        socket.close();
        Timer::after(embassy_time::Duration::from_millis(
            self.options.socket_close_delay.as_millis(),
        ))
        .await;

        result
    }

    /// The delay between read retries, as a future.
    fn retry_delay(&self) -> impl Future<Output = ()> {
        Timer::after(embassy_time::Duration::from_millis(
            self.options.retry_delay.as_millis(),
        ))
    }

    /// Make HTTP request with zero-copy response handling
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
    async fn make_http_request<'b>(
        &self,
        method: HttpMethod,
        endpoint: &HttpEndpoint<'_>,
        headers: &[HttpHeader<'_>],
        body: Option<&[u8]>,
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error> {
        let host_header = endpoint.host_header::<HOST_HEADER_SIZE>()?;
        let client = BaseHttpClient::<RQ>::with_options(self.options);
        self.with_http_connection(endpoint, async |socket| {
            client
                .request_with_retry_delay(
                    socket,
                    HttpClientRequest {
                        method,
                        host: &host_header,
                        path: endpoint.path,
                        query: endpoint.query,
                        headers,
                        body,
                    },
                    response_buffer,
                    || self.retry_delay(),
                )
                .await
        })
        .await
    }

    /// Make a request and stream the response body to `sink` instead of
    /// collecting it in a buffer.
    ///
    /// `buffer` only has to hold the status line and headers plus the piece of
    /// the body read at a time. See
    /// [`HttpClient::request_streaming`](nanofish::HttpClient::request_streaming)
    /// for how the sink is driven and which errors can occur. Redirects are not
    /// followed; check the status in [`BodySink::head`] and request the
    /// `Location` yourself. [`HttpClientOptions::total_timeout`] applies.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`EmbassyHttpClient::request`] that apply before
    /// the response, and those of `request_streaming` while reading it.
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
    pub async fn request_streaming<K: BodySink>(
        &self,
        method: HttpMethod,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        body: Option<&[u8]>,
        buffer: &mut [u8],
        sink: &mut K,
    ) -> Result<StreamedResponse, Error> {
        let attempt = async {
            let endpoint = parse_endpoint(endpoint)?;
            let host_header = endpoint.host_header::<HOST_HEADER_SIZE>()?;
            let client = BaseHttpClient::<RQ>::with_options(self.options);
            let request = HttpClientRequest {
                method,
                host: &host_header,
                path: endpoint.path,
                query: endpoint.query,
                headers,
                body,
            };

            match endpoint.scheme {
                "https" => {
                    self.https_streaming(&endpoint, &client, request, buffer, sink)
                        .await
                }
                "http" => {
                    self.with_http_connection(&endpoint, async |socket| {
                        client
                            .request_streaming_with_retry_delay(
                                socket,
                                request,
                                buffer,
                                sink,
                                || self.retry_delay(),
                            )
                            .await
                    })
                    .await
                }
                _ => Err(Error::UnsupportedScheme("unknown")),
            }
        };

        match self.options.total_timeout {
            Some(limit) => with_timeout(
                embassy_time::Duration::from_millis(limit.as_millis()),
                attempt,
            )
            .await
            .unwrap_or(Err(Error::Timeout)),
            None => attempt.await,
        }
    }

    /// Send a request described by a [`RequestBuilder`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::BufferOverflow`] if the builder was given too many
    /// headers, otherwise the same errors as [`EmbassyHttpClient::request`].
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
    pub async fn send<'b>(
        &self,
        endpoint: &str,
        builder: &RequestBuilder<'_>,
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error> {
        builder.check()?;
        self.request(
            builder.method(),
            endpoint,
            builder.headers(),
            builder.body_bytes(),
            response_buffer,
        )
        .await
    }

    /// Convenience method for making a POST request with a JSON body. Adds
    /// `Content-Type: application/json`.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`EmbassyHttpClient::send`].
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
    pub async fn post_json<'b>(
        &self,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        json: &str,
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error> {
        let builder = RequestBuilder::post().headers_from(headers).json(json);
        self.send(endpoint, &builder, response_buffer).await
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
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
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
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
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
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
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
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
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
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
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
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
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
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
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
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
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
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
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

/// Resolve a hostname to an IP address, trying IPv4 (A) first then IPv6 (AAAA).
#[expect(
    clippy::future_not_send,
    reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
)]
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

/// A persistent HTTP connection to one server, reused for several requests.
///
/// [`EmbassyHttpClient`] opens and closes a connection for every request, which
/// costs a DNS lookup, a TCP handshake and the configured
/// [`HttpClientOptions::socket_close_delay`] each time. A session connects
/// once and keeps the socket open (HTTP/1.1 keep-alive) while the server does.
///
/// It reconnects by itself when the server closed the connection, either
/// because it said `Connection: close` or because it dropped an idle
/// connection. A request that fails on a reused connection before any response
/// byte arrives is repeated once on a fresh connection, but only for methods
/// that are safe to repeat (`GET`, `HEAD`, `PUT`, `DELETE`, `OPTIONS` and
/// `TRACE`).
///
/// Sessions speak plain HTTP; use [`EmbassyHttpClient`] for `https` URLs.
pub struct EmbassyHttpSession<'a, const RQ: usize = REQUEST_SIZE> {
    socket: TcpSocket<'a>,
    options: HttpClientOptions,
    remote: (embassy_net::IpAddress, u16),
    host_header: String<HOST_HEADER_SIZE>,
    /// The socket is connected and the last response allowed reuse.
    open: bool,
}

impl<'a, const RQ: usize> EmbassyHttpSession<'a, RQ> {
    /// Connect to the server of `endpoint` (an `http` URL; its path is ignored).
    ///
    /// `rx_buffer` and `tx_buffer` back the TCP socket for the whole life of
    /// the session.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnsupportedScheme`] for anything but `http`, and the
    /// DNS and connection errors of [`EmbassyHttpClient::request`].
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
    pub async fn connect(
        stack: Stack<'a>,
        rx_buffer: &'a mut [u8],
        tx_buffer: &'a mut [u8],
        endpoint: &str,
        options: HttpClientOptions,
    ) -> Result<Self, Error> {
        let endpoint = parse_endpoint(endpoint)?;
        if endpoint.scheme != "http" {
            return Err(Error::UnsupportedScheme("sessions support only http"));
        }
        let host_header = endpoint.host_header::<HOST_HEADER_SIZE>()?;
        let ip_addr = resolve_host(stack, endpoint.host).await?;

        let mut socket = TcpSocket::new(stack, rx_buffer, tx_buffer);
        socket.set_timeout(Some(embassy_time::Duration::from_millis(
            options.socket_timeout.as_millis(),
        )));
        let mut session = Self {
            socket,
            options,
            remote: (ip_addr, endpoint.port),
            host_header,
            open: false,
        };
        session.reconnect().await?;
        Ok(session)
    }

    /// Whether the connection is believed to be usable for the next request.
    ///
    /// It can still turn out to be closed by the server; [`request`](Self::request)
    /// copes with that.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.open
    }

    /// (Re)open the connection.
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
    async fn reconnect(&mut self) -> Result<(), Error> {
        self.open = false;
        self.socket.abort();
        // Let the reset go out before the socket is reused.
        let _ = self.socket.flush().await;
        self.socket
            .connect(self.remote)
            .await
            .map_err(|e: embassy_net::tcp::ConnectError| {
                self.socket.abort();
                Error::from(e)
            })?;
        self.open = true;
        Ok(())
    }

    /// Send one request on the session, connecting again first if needed.
    ///
    /// `target` is the path and optional query, for example `/api?limit=10`.
    /// The `Host` header is that of the session's server.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`EmbassyHttpClient::request`], and
    /// [`Error::Timeout`] if [`HttpClientOptions::total_timeout`] is exceeded.
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
    pub async fn request<'b>(
        &mut self,
        method: HttpMethod,
        target: &str,
        headers: &[HttpHeader<'_>],
        body: Option<&[u8]>,
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error> {
        let total_timeout = self.options.total_timeout;
        let attempt = self.request_inner(method, target, headers, body, response_buffer);
        let used = match total_timeout {
            Some(limit) => {
                let result = with_timeout(
                    embassy_time::Duration::from_millis(limit.as_millis()),
                    attempt,
                )
                .await
                .unwrap_or(Err(Error::Timeout));
                if result.is_err() {
                    // A half-finished exchange leaves the connection unusable.
                    self.open = false;
                }
                result?
            }
            None => attempt.await?,
        };
        // The response is parsed again from the buffer that now holds it.
        Ok((codec::parse_resp(&response_buffer[..used])?, used))
    }

    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
    async fn request_inner(
        &mut self,
        method: HttpMethod,
        target: &str,
        headers: &[HttpHeader<'_>],
        body: Option<&[u8]>,
        response_buffer: &mut [u8],
    ) -> Result<usize, Error> {
        let (path, query) = target
            .split_once('?')
            .map_or((target, None), |(path, query)| (path, Some(query)));
        let client = BaseHttpClient::<RQ>::with_options(self.options);

        let mut reused = self.open;
        loop {
            if !self.open {
                self.reconnect().await?;
            }
            let host_header = self.host_header.clone();
            let request = HttpClientRequest {
                method,
                host: &host_header,
                path,
                query,
                headers,
                body,
            };
            let outcome = client
                .exchange_with_retry_delay(
                    &mut self.socket,
                    request,
                    &mut *response_buffer,
                    true,
                    || {
                        Timer::after(embassy_time::Duration::from_millis(
                            self.options.retry_delay.as_millis(),
                        ))
                    },
                )
                .await
                .map(|exchange| (exchange.used, exchange.reusable));

            match outcome {
                Ok((used, reusable)) => {
                    self.open = reusable;
                    return Ok(used);
                }
                // The server may have dropped an idle connection: repeat the
                // request once on a new one, if repeating it is harmless.
                Err(Error::NoResponse | Error::TcpError(_)) if reused && is_repeatable(method) => {
                    self.open = false;
                    reused = false;
                }
                Err(error) => {
                    self.open = false;
                    return Err(error);
                }
            }
        }
    }

    /// Close the connection.
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
    pub async fn close(mut self) {
        self.socket.close();
        Timer::after(embassy_time::Duration::from_millis(
            self.options.socket_close_delay.as_millis(),
        ))
        .await;
    }
}

/// Methods that can be sent again after a connection failed without a
/// response (RFC 9110 section 9.2.2).
const fn is_repeatable(method: HttpMethod) -> bool {
    matches!(
        method,
        HttpMethod::GET
            | HttpMethod::HEAD
            | HttpMethod::PUT
            | HttpMethod::DELETE
            | HttpMethod::OPTIONS
            | HttpMethod::TRACE
    )
}

#[cfg(test)]
mod tests {
    use embassy_net::Stack;
    use nanofish::TimeoutDuration;
    use nanofish::client::embassy::*;

    #[test]
    fn test_repeatable_methods() {
        for method in [
            HttpMethod::GET,
            HttpMethod::HEAD,
            HttpMethod::PUT,
            HttpMethod::DELETE,
            HttpMethod::OPTIONS,
            HttpMethod::TRACE,
        ] {
            assert!(is_repeatable(method), "{method:?}");
        }
        for method in [HttpMethod::POST, HttpMethod::PATCH, HttpMethod::CONNECT] {
            assert!(!is_repeatable(method), "{method:?}");
        }
    }

    #[test]
    fn test_new_and_with_options() {
        // This test only checks that the options are set correctly, not that the stack is valid.
        // Use a raw pointer to avoid UB and static mut issues. This is safe for type-checking only.
        let fake_stack: *const Stack = core::ptr::NonNull::dangling().as_ptr();
        let client = DefaultEmbassyHttpClient::new(unsafe { &*fake_stack });
        let opts = HttpClientOptions {
            max_retries: 1,
            socket_timeout: TimeoutDuration::from_secs(1),
            total_timeout: None,
            max_redirects: 0,
            retry_delay: TimeoutDuration::from_millis(1),
            socket_close_delay: TimeoutDuration::from_millis(1),
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
                socket_timeout: TimeoutDuration::from_secs(2),
                total_timeout: None,
                max_redirects: 0,
                retry_delay: TimeoutDuration::from_millis(10),
                socket_close_delay: TimeoutDuration::from_millis(5),
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
                socket_timeout: TimeoutDuration::from_secs(1),
                total_timeout: None,
                max_redirects: 0,
                retry_delay: TimeoutDuration::from_millis(5),
                socket_close_delay: TimeoutDuration::from_millis(2),
            },
        );
        assert_eq!(client_small_custom.options.max_retries, 2);
    }
}
