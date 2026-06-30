//! Transport-generic client and server helpers built on `embedded-io-async`.
//!
//! This module is available without the `embassy` feature. It lets callers run
//! Nanofish over any already-connected async stream, while platform-specific
//! DNS, TCP accept/connect, and timeout behavior remain outside the core crate.

use crate::{
    codec,
    error::Error,
    handler::HttpHandler,
    header::{HttpHeader, headers::CONTENT_LENGTH, mime_types},
    method::HttpMethod,
    options::HttpClientOptions,
    protocol::{self, DOUBLE_CRLF_LEN},
    request::HttpRequest,
    response::{HttpResponse, ResponseBody},
    status_code::StatusCode,
};
use embedded_io_async::{Read, Write};
use heapless::Vec;

const DEFAULT_REQUEST_SIZE: usize = 1024;
const SMALL_REQUEST_SIZE: usize = 1024;
const DEFAULT_SERVER_REQUEST_SIZE: usize = 4096;
const DEFAULT_SERVER_RESPONSE_SIZE: usize = 4096;
const SMALL_SERVER_RESPONSE_SIZE: usize = 1024;

/// Request metadata for [`HttpIoClient`].
pub struct HttpIoRequest<'a> {
    /// HTTP method to send.
    pub method: HttpMethod,
    /// Host value used for the HTTP `Host` header.
    pub host: &'a str,
    /// Request target, for example `/`, `/api`, or `/api?limit=10`.
    pub path: &'a str,
    /// Additional request headers.
    pub headers: &'a [HttpHeader<'a>],
    /// Optional request body.
    pub body: Option<&'a [u8]>,
}

/// Transport-generic HTTP client for already-connected streams.
///
/// This client does not perform DNS resolution, TCP connection setup, TLS, or
/// sleeping between retries. It writes an HTTP request to the provided stream,
/// reads the response into the caller-provided buffer, and parses the response
/// with zero-copy body references.
pub struct HttpIoClient<const RQ: usize = DEFAULT_REQUEST_SIZE> {
    options: HttpClientOptions,
}

impl HttpIoClient<DEFAULT_REQUEST_SIZE> {
    /// Create a new transport-generic client with default options and buffer sizes.
    #[must_use]
    pub fn new() -> Self {
        Self {
            options: HttpClientOptions::default(),
        }
    }
}

impl<const RQ: usize> HttpIoClient<RQ> {
    /// Create a new transport-generic client with custom options.
    #[must_use]
    pub const fn with_options(options: HttpClientOptions) -> Self {
        Self { options }
    }

    /// Send one HTTP request over an already-connected stream.
    ///
    /// `host` is used for the HTTP `Host` header. `path` must be the request
    /// target, for example `/`, `/api`, or `/api?limit=10`.
    ///
    /// # Errors
    ///
    /// Returns an error if request construction fails, stream IO fails, no
    /// response is received, or the response cannot be parsed.
    pub async fn request<'b, S>(
        &self,
        stream: &mut S,
        request: HttpIoRequest<'_>,
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error>
    where
        S: Read + Write,
    {
        let http_request = codec::build_req::<RQ>(
            request.method,
            request.host,
            request.path,
            request.headers,
            request.body,
        )?;

        stream
            .write_all(http_request.as_bytes())
            .await
            .map_err(|_| Error::TcpError)?;

        if let Some(body_data) = request.body {
            stream
                .write_all(body_data)
                .await
                .map_err(|_| Error::TcpError)?;
        }

        stream.flush().await.map_err(|_| Error::TcpError)?;

        let total_read = read_response(stream, response_buffer, self.options.max_retries).await?;
        let total_read = codec::dechunk(response_buffer, total_read)?;
        let response = codec::parse_resp(&response_buffer[..total_read])?;
        Ok((response, total_read))
    }

    /// Convenience method for making a GET request over an already-connected stream.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpIoClient::request`].
    pub async fn get<'b, S>(
        &self,
        stream: &mut S,
        host: &str,
        path: &str,
        headers: &[HttpHeader<'_>],
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error>
    where
        S: Read + Write,
    {
        self.request(
            stream,
            HttpIoRequest {
                method: HttpMethod::GET,
                host,
                path,
                headers,
                body: None,
            },
            response_buffer,
        )
        .await
    }

    /// Convenience method for making a POST request over an already-connected stream.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpIoClient::request`].
    pub async fn post<'b, S>(
        &self,
        stream: &mut S,
        host: &str,
        path: &str,
        headers: &[HttpHeader<'_>],
        body: &[u8],
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error>
    where
        S: Read + Write,
    {
        self.request(
            stream,
            HttpIoRequest {
                method: HttpMethod::POST,
                host,
                path,
                headers,
                body: Some(body),
            },
            response_buffer,
        )
        .await
    }
}

impl Default for HttpIoClient<DEFAULT_REQUEST_SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

/// Type alias for `HttpIoClient` with the default request buffer size.
pub type DefaultHttpIoClient = HttpIoClient<DEFAULT_REQUEST_SIZE>;

/// Type alias for `HttpIoClient` with a smaller request buffer size.
pub type SmallHttpIoClient = HttpIoClient<SMALL_REQUEST_SIZE>;

/// Transport-generic HTTPS client for already-connected streams.
///
/// This is available with the `tls` feature and does not require Embassy. The
/// caller supplies an already-connected TCP-like stream and a cryptographic RNG.
#[cfg(feature = "tls")]
pub struct HttpTlsIoClient<
    const RQ: usize = DEFAULT_REQUEST_SIZE,
    const TLS_READ: usize = 4096,
    const TLS_WRITE: usize = 4096,
> {
    options: HttpClientOptions,
}

#[cfg(feature = "tls")]
impl HttpTlsIoClient<DEFAULT_REQUEST_SIZE, 4096, 4096> {
    /// Create a new transport-generic TLS client with default buffer sizes.
    #[must_use]
    pub fn new() -> Self {
        Self {
            options: HttpClientOptions::default(),
        }
    }
}

#[cfg(feature = "tls")]
impl<const RQ: usize, const TLS_READ: usize, const TLS_WRITE: usize>
    HttpTlsIoClient<RQ, TLS_READ, TLS_WRITE>
{
    /// Create a new transport-generic TLS client with custom options.
    #[must_use]
    pub const fn with_options(options: HttpClientOptions) -> Self {
        Self { options }
    }

    /// Send one HTTPS request over an already-connected stream.
    ///
    /// `server_name` is used for the TLS server name indication. DNS, TCP
    /// connection setup, timeouts, and entropy sourcing are caller/platform
    /// responsibilities.
    ///
    /// # Errors
    ///
    /// Returns an error if TLS handshake/IO fails, request construction fails,
    /// no response is received, or the response cannot be parsed.
    #[expect(clippy::future_not_send)]
    pub async fn request<'b, S, RNG>(
        &self,
        stream: S,
        server_name: &str,
        request: HttpIoRequest<'_>,
        response_buffer: &'b mut [u8],
        rng: RNG,
    ) -> Result<(HttpResponse<'b>, usize), Error>
    where
        S: Read + Write,
        RNG: embedded_tls::CryptoRngCore,
    {
        let tls_config = embedded_tls::TlsConfig::new().with_server_name(server_name);
        let mut read_record_buffer = [0; TLS_READ];
        let mut write_record_buffer = [0; TLS_WRITE];
        let mut tls = embedded_tls::TlsConnection::new(
            stream,
            &mut read_record_buffer,
            &mut write_record_buffer,
        );

        tls.open(embedded_tls::TlsContext::new(
            &tls_config,
            embedded_tls::UnsecureProvider::new::<embedded_tls::Aes128GcmSha256>(rng),
        ))
        .await?;

        let client = HttpIoClient::<RQ>::with_options(self.options);
        let result = client.request(&mut tls, request, response_buffer).await;
        let _ = tls.close().await;
        result
    }
}

#[cfg(feature = "tls")]
impl Default for HttpTlsIoClient<DEFAULT_REQUEST_SIZE, 4096, 4096> {
    fn default() -> Self {
        Self::new()
    }
}

/// Type alias for `HttpTlsIoClient` with default request and TLS buffer sizes.
#[cfg(feature = "tls")]
pub type DefaultHttpTlsIoClient = HttpTlsIoClient<DEFAULT_REQUEST_SIZE, 4096, 4096>;

/// Type alias for `HttpTlsIoClient` with smaller request and TLS buffer sizes.
#[cfg(feature = "tls")]
pub type SmallHttpTlsIoClient = HttpTlsIoClient<SMALL_REQUEST_SIZE, 1024, 1024>;

/// Transport-generic HTTP server for already-accepted streams.
///
/// This is the non-Embassy counterpart to the Embassy-backed server. It handles
/// one request/response cycle per call and leaves accept loops, timeouts, and
/// connection lifecycle to the caller.
pub struct HttpIoServer<
    const REQ_SIZE: usize = DEFAULT_SERVER_REQUEST_SIZE,
    const MAX_RESPONSE_SIZE: usize = DEFAULT_SERVER_RESPONSE_SIZE,
>;

impl HttpIoServer<DEFAULT_SERVER_REQUEST_SIZE, DEFAULT_SERVER_RESPONSE_SIZE> {
    /// Create a new transport-generic server with default buffer sizes.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl<const REQ_SIZE: usize, const MAX_RESPONSE_SIZE: usize>
    HttpIoServer<REQ_SIZE, MAX_RESPONSE_SIZE>
{
    /// Create a new transport-generic server with custom buffer sizes.
    #[must_use]
    pub const fn with_buffer_sizes() -> Self {
        Self
    }

    /// Handle one request/response cycle over an already-accepted stream.
    ///
    /// # Errors
    ///
    /// Returns an error if stream IO fails, request parsing fails, handler execution
    /// fails, or response serialization exceeds the configured response buffer.
    pub async fn handle_connection<S, H>(
        &self,
        stream: &mut S,
        handler: &mut H,
    ) -> Result<(), Error>
    where
        S: Read + Write,
        H: HttpHandler,
    {
        handle_http_connection_with_sizes::<S, H, REQ_SIZE, MAX_RESPONSE_SIZE>(stream, handler)
            .await
    }
}

impl Default for HttpIoServer<DEFAULT_SERVER_REQUEST_SIZE, DEFAULT_SERVER_RESPONSE_SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

/// Type alias for `HttpIoServer` with default request and response buffer sizes.
pub type DefaultHttpIoServer =
    HttpIoServer<DEFAULT_SERVER_REQUEST_SIZE, DEFAULT_SERVER_RESPONSE_SIZE>;

/// Type alias for `HttpIoServer` with smaller request and response buffer sizes.
pub type SmallHttpIoServer = HttpIoServer<SMALL_REQUEST_SIZE, SMALL_SERVER_RESPONSE_SIZE>;

/// Handle a single HTTP server connection over a generic async stream.
///
/// This function reads one complete HTTP request, invokes `handler`, writes the
/// response, and flushes the stream. Accept loops, timeouts, and connection
/// lifecycle are intentionally left to the caller or platform adapter.
///
/// # Errors
///
/// Returns an error if stream IO fails, request parsing fails, handler execution
/// fails, or response serialization exceeds the configured response buffer.
pub async fn handle_http_connection<S, H>(stream: &mut S, handler: &mut H) -> Result<(), Error>
where
    S: Read + Write,
    H: HttpHandler,
{
    handle_http_connection_with_sizes::<
        S,
        H,
        DEFAULT_SERVER_REQUEST_SIZE,
        DEFAULT_SERVER_RESPONSE_SIZE,
    >(stream, handler)
    .await
}

/// Handle a single HTTP server connection with custom request/response buffer sizes.
///
/// # Errors
///
/// Returns an error if stream IO fails, request parsing fails, handler execution
/// fails, or response serialization exceeds the configured response buffer.
pub async fn handle_http_connection_with_sizes<
    S,
    H,
    const REQ_SIZE: usize,
    const MAX_RESPONSE_SIZE: usize,
>(
    stream: &mut S,
    handler: &mut H,
) -> Result<(), Error>
where
    S: Read + Write,
    H: HttpHandler,
{
    let mut request_buffer = [0; REQ_SIZE];
    let total_read = read_request(stream, &mut request_buffer).await?;
    if total_read == 0 {
        return Err(Error::NoResponse);
    }

    let request = HttpRequest::try_from(&request_buffer[..total_read])?;
    let response = handler.handle_request(&request).await.map_or_else(
        |_| {
            text_error_response::<MAX_RESPONSE_SIZE>(
                StatusCode::InternalServerError,
                "Internal Server Error",
            )
        },
        |response| response.build_bytes::<MAX_RESPONSE_SIZE>(),
    )?;

    stream
        .write_all(&response)
        .await
        .map_err(|_| Error::TcpError)?;
    stream.flush().await.map_err(|_| Error::TcpError)
}

async fn read_response<S>(
    stream: &mut S,
    response_buffer: &mut [u8],
    max_retries: usize,
) -> Result<usize, Error>
where
    S: Read,
{
    let mut total_read = 0;
    let mut retries = max_retries;

    while total_read < response_buffer.len() && retries > 0 {
        match stream.read(&mut response_buffer[total_read..]).await {
            Ok(0) => break,
            Ok(n) => {
                total_read += n;
                if codec::complete(&response_buffer[..total_read]) {
                    break;
                }
            }
            Err(_) => {
                retries -= 1;
                if retries == 0 {
                    return Err(Error::TcpError);
                }
            }
        }
    }

    if total_read == 0 {
        return Err(Error::NoResponse);
    }

    Ok(total_read)
}

async fn read_request<S>(stream: &mut S, buf: &mut [u8]) -> Result<usize, Error>
where
    S: Read,
{
    let mut total_read = 0;
    let mut header_end = None;

    while total_read < buf.len() {
        let n = stream
            .read(&mut buf[total_read..])
            .await
            .map_err(|_| Error::TcpError)?;
        if n == 0 {
            break;
        }
        total_read += n;

        if header_end.is_none() {
            header_end = protocol::find_double_crlf(&buf[..total_read]);
        }

        if let Some(hdr_end) = header_end {
            let body_start = hdr_end + DOUBLE_CRLF_LEN;
            if let Some(content_length) = parse_content_length(&buf[..hdr_end]) {
                if total_read >= body_start + content_length {
                    break;
                }
            } else {
                break;
            }
        }
    }

    Ok(total_read)
}

fn parse_content_length(header_bytes: &[u8]) -> Option<usize> {
    let headers_str = core::str::from_utf8(header_bytes).ok()?;
    protocol::find_header_value(headers_str, CONTENT_LENGTH)?
        .parse()
        .ok()
}

fn text_error_response<const MAX_RESPONSE_SIZE: usize>(
    status: StatusCode,
    body: &str,
) -> Result<Vec<u8, MAX_RESPONSE_SIZE>, Error> {
    let mut headers = Vec::new();
    let _ = headers.push(HttpHeader::content_type(mime_types::TEXT));
    let resp = HttpResponse {
        status_code: status,
        headers,
        body: ResponseBody::Text(body),
    };
    resp.build_bytes::<MAX_RESPONSE_SIZE>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ResponseBody, SimpleHandler};
    use core::convert::Infallible;
    use embedded_io_async::ErrorType;

    struct MockStream<const IN: usize, const OUT: usize> {
        input: Vec<u8, IN>,
        output: Vec<u8, OUT>,
        read_pos: usize,
    }

    impl<const IN: usize, const OUT: usize> MockStream<IN, OUT> {
        fn new(input: &[u8]) -> Self {
            Self {
                input: Vec::from_slice(input).unwrap(),
                output: Vec::new(),
                read_pos: 0,
            }
        }
    }

    impl<const IN: usize, const OUT: usize> ErrorType for MockStream<IN, OUT> {
        type Error = Infallible;
    }

    impl<const IN: usize, const OUT: usize> Read for MockStream<IN, OUT> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            if self.read_pos >= self.input.len() {
                return Ok(0);
            }
            let n = buf.len().min(self.input.len() - self.read_pos);
            buf[..n].copy_from_slice(&self.input[self.read_pos..self.read_pos + n]);
            self.read_pos += n;
            Ok(n)
        }
    }

    impl<const IN: usize, const OUT: usize> Write for MockStream<IN, OUT> {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            self.output.extend_from_slice(buf).unwrap();
            Ok(buf.len())
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[test]
    fn test_io_client_get() {
        let response =
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nhello";
        let mut stream = MockStream::<128, 256>::new(response);
        let client = HttpIoClient::new();
        let mut buffer = [0; 128];

        let (response, _) = futures_lite::future::block_on(client.get(
            &mut stream,
            "example.com",
            "/hello",
            &[],
            &mut buffer,
        ))
        .unwrap();

        assert_eq!(response.status_code, StatusCode::Ok);
        assert_eq!(response.body.as_str(), Some("hello"));
        let request = core::str::from_utf8(&stream.output).unwrap();
        assert!(request.starts_with("GET /hello HTTP/1.1\r\nHost: example.com\r\n"));
    }

    #[test]
    fn test_handle_http_connection() {
        let request = b"GET /health HTTP/1.1\r\nHost: example.com\r\n\r\n";
        let mut stream = MockStream::<128, 512>::new(request);
        let mut handler = SimpleHandler;

        futures_lite::future::block_on(handle_http_connection_with_sizes::<_, _, 128, 512>(
            &mut stream,
            &mut handler,
        ))
        .unwrap();

        let response = core::str::from_utf8(&stream.output).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.contains("{\"status\":\"ok\"}"));
    }

    #[test]
    fn test_io_server_handle_connection() {
        let request = b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\n";
        let mut stream = MockStream::<128, 512>::new(request);
        let mut handler = SimpleHandler;
        let server = HttpIoServer::<128, 512>::with_buffer_sizes();

        futures_lite::future::block_on(server.handle_connection(&mut stream, &mut handler))
            .unwrap();

        let response = core::str::from_utf8(&stream.output).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.contains("Hello from nanofish HTTP server"));
    }

    #[test]
    fn test_io_client_binary_response() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 3\r\n\r\n\x01\x02\x03";
        let mut stream = MockStream::<128, 256>::new(response);
        let client = HttpIoClient::new();
        let mut buffer = [0; 128];

        let (response, _) = futures_lite::future::block_on(client.get(
            &mut stream,
            "example.com",
            "/bin",
            &[],
            &mut buffer,
        ))
        .unwrap();

        assert_eq!(response.body, ResponseBody::Binary(&[1, 2, 3]));
    }
}
