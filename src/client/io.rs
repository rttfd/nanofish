use crate::{
    codec,
    error::Error,
    header::HttpHeader,
    method::HttpMethod,
    options::HttpClientOptions,
    protocol::{DEFAULT_HTTP_PORT, DEFAULT_HTTPS_PORT},
    response::HttpResponse,
};
use core::future::Future;
use embedded_io_async::{Read, Write};

use crate::client::{DEFAULT_REQUEST_SIZE, SMALL_REQUEST_SIZE};

/// Parsed HTTP endpoint metadata.
pub struct HttpEndpoint<'a> {
    /// URL scheme, either `http` or `https`.
    pub scheme: &'a str,
    /// Hostname without port.
    pub host: &'a str,
    /// Explicit or default port.
    pub port: u16,
    /// Request path, defaulting to `/`.
    pub path: &'a str,
}

/// Parse an HTTP or HTTPS URL into endpoint metadata.
///
/// # Errors
///
/// Returns [`Error::InvalidUrl`] if the endpoint does not start with `http://` or `https://`.
pub fn parse_endpoint(endpoint: &str) -> Result<HttpEndpoint<'_>, Error> {
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
    let (host, port) = host.rfind(':').map_or((host, default_port), |colon_pos| {
        host[colon_pos + 1..]
            .parse::<u16>()
            .map_or((host, default_port), |port| (&host[..colon_pos], port))
    });

    Ok(HttpEndpoint {
        scheme,
        host,
        port,
        path,
    })
}

/// Request metadata for [`HttpClient`].
pub struct HttpClientRequest<'a> {
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
pub struct HttpClient<const RQ: usize = DEFAULT_REQUEST_SIZE> {
    options: HttpClientOptions,
}
/// Type alias for `HttpClient` with the default request buffer size.
pub type DefaultHttpClient = HttpClient<DEFAULT_REQUEST_SIZE>;

/// Type alias for `HttpClient` with a smaller request buffer size (512 bytes).
pub type SmallHttpClient = HttpClient<SMALL_REQUEST_SIZE>;

impl Default for HttpClient<DEFAULT_REQUEST_SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpClient<DEFAULT_REQUEST_SIZE> {
    /// Create a new transport-generic client with default options and buffer sizes.
    #[must_use]
    pub fn new() -> Self {
        Self {
            options: HttpClientOptions::default(),
        }
    }
}

impl<const RQ: usize> HttpClient<RQ> {
    /// Create a new transport-generic client with custom options.
    #[must_use]
    pub const fn with_options(options: HttpClientOptions) -> Self {
        Self { options }
    }

    /// Send one HTTP request over an already-connected stream.
    ///
    /// Read errors are retried up to [`HttpClientOptions::max_retries`] times
    /// without pausing between attempts. Use
    /// [`HttpClient::request_with_retry_delay`] to sleep between retries.
    ///
    /// # Errors
    ///
    /// Returns an error if request construction fails, stream IO fails, no
    /// response is received, the response is incomplete or larger than the
    /// response buffer, or the response cannot be parsed.
    pub async fn request<'b, S>(
        &self,
        stream: &mut S,
        request: HttpClientRequest<'_>,
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error>
    where
        S: Read + Write,
    {
        self.request_with_retry_delay(stream, request, response_buffer, || core::future::ready(()))
            .await
    }

    /// Send one HTTP request over an already-connected stream, sleeping via
    /// `retry_delay` between read retries.
    ///
    /// This is the transport-neutral counterpart used by Embassy-backed
    /// adapters to honor [`HttpClientOptions::retry_delay`]; callers without a
    /// timer can use [`HttpClient::request`] instead.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpClient::request`].
    pub async fn request_with_retry_delay<'b, S, D, Fut>(
        &self,
        stream: &mut S,
        request: HttpClientRequest<'_>,
        response_buffer: &'b mut [u8],
        retry_delay: D,
    ) -> Result<(HttpResponse<'b>, usize), Error>
    where
        S: Read + Write,
        D: Fn() -> Fut,
        Fut: Future<Output = ()>,
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

        let total_read = read_response(
            stream,
            response_buffer,
            self.options.max_retries,
            retry_delay,
        )
        .await?;
        let total_read = codec::dechunk(response_buffer, total_read)?;
        let response = codec::parse_resp(&response_buffer[..total_read])?;
        Ok((response, total_read))
    }

    /// Convenience method for making a GET request over an already-connected stream.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpClient::request`].
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
            HttpClientRequest {
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
    /// Returns the same errors as [`HttpClient::request`].
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
            HttpClientRequest {
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

/// Read a full HTTP response into `buffer`, retrying transient read errors.
///
/// `retry_delay` is awaited between retries. A response is complete once its
/// headers plus `Content-Length` body bytes or the chunked terminator arrive;
/// a body without either is delimited by connection close.
///
/// # Errors
///
/// - [`Error::NoResponse`] if nothing was read.
/// - [`Error::TcpError`] if `max_retries` read attempts fail.
/// - [`Error::InvalidResponse`] if the peer closes the connection before a
///   length-delimited (or chunked) body is complete.
/// - [`Error::BufferOverflow`] if the buffer fills before the response is
///   complete.
async fn read_response<S, D, Fut>(
    stream: &mut S,
    response_buffer: &mut [u8],
    max_retries: usize,
    retry_delay: D,
) -> Result<usize, Error>
where
    S: Read,
    D: Fn() -> Fut,
    Fut: Future<Output = ()>,
{
    let mut total_read = 0;
    let mut retries = max_retries;
    let mut saw_eof = false;

    while total_read < response_buffer.len() && retries > 0 {
        match stream.read(&mut response_buffer[total_read..]).await {
            Ok(0) => {
                saw_eof = true;
                break;
            }
            Ok(n) => {
                total_read += n;
                if codec::complete(&response_buffer[..total_read]) {
                    return Ok(total_read);
                }
            }
            Err(_) => {
                retries -= 1;
                if retries == 0 {
                    return Err(Error::TcpError);
                }
                retry_delay().await;
            }
        }
    }

    if total_read == 0 {
        return Err(Error::NoResponse);
    }

    if codec::complete(&response_buffer[..total_read]) {
        return Ok(total_read);
    }

    if saw_eof {
        if codec::declares_length(&response_buffer[..total_read]) {
            return Err(Error::InvalidResponse("Incomplete response body"));
        }
        // No length was declared: connection close delimits the body.
        return Ok(total_read);
    }

    // The buffer filled before the response was complete.
    Err(Error::BufferOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ResponseBody, StatusCode};
    use core::convert::Infallible;
    use embedded_io_async::ErrorType;
    use heapless::Vec;

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
        let client = HttpClient::new();
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
    fn test_io_client_binary_response() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 3\r\n\r\n\x01\x02\x03";
        let mut stream = MockStream::<128, 256>::new(response);
        let client = HttpClient::new();
        let mut buffer = [0; 128];

        let (response, _) = futures_lite::future::block_on(client.get(
            &mut stream,
            "example.com",
            "/bin",
            &[],
            &mut buffer,
        ))
        .unwrap();

        assert_eq!(response.status_code, StatusCode::Ok);
        assert_eq!(response.body, ResponseBody::Binary(&[1, 2, 3]));
    }

    #[test]
    fn test_io_client_post_with_body() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
        let mut stream = MockStream::<128, 256>::new(response);
        let client = HttpClient::new();
        let mut buffer = [0; 64];

        let (response, _) = futures_lite::future::block_on(client.post(
            &mut stream,
            "example.com",
            "/submit",
            &[],
            b"hello",
            &mut buffer,
        ))
        .unwrap();

        assert_eq!(response.status_code, StatusCode::Ok);
        let request = core::str::from_utf8(&stream.output).unwrap();
        assert!(request.starts_with("POST /submit HTTP/1.1\r\nHost: example.com\r\n"));
        assert!(request.contains("Content-Length: 5\r\n"));
        assert!(request.ends_with("hello"));
    }

    #[test]
    fn test_io_client_post_explicit_content_length_not_duplicated() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
        let mut stream = MockStream::<128, 256>::new(response);
        let client = HttpClient::new();
        let mut buffer = [0; 64];

        futures_lite::future::block_on(client.post(
            &mut stream,
            "example.com",
            "/submit",
            &[HttpHeader::new("Content-Length", "5")],
            b"hello",
            &mut buffer,
        ))
        .unwrap();

        let request = core::str::from_utf8(&stream.output).unwrap();
        assert_eq!(request.matches("Content-Length").count(), 1);
    }

    #[test]
    fn test_io_client_response_larger_than_buffer() {
        let mut raw = [0u8; 140];
        raw[..40].copy_from_slice(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n");
        for byte in &mut raw[40..] {
            *byte = b'x';
        }
        let mut stream = MockStream::<256, 256>::new(&raw);
        let client = HttpClient::new();
        let mut buffer = [0; 64];

        let result = futures_lite::future::block_on(client.get(
            &mut stream,
            "example.com",
            "/",
            &[],
            &mut buffer,
        ));

        assert!(matches!(result, Err(Error::BufferOverflow)));
    }

    #[test]
    fn test_io_client_premature_close_with_content_length() {
        // Declares Content-Length: 100 but closes after 3 body bytes.
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nabc";
        let mut stream = MockStream::<128, 256>::new(response);
        let client = HttpClient::new();
        let mut buffer = [0; 128];

        let result = futures_lite::future::block_on(client.get(
            &mut stream,
            "example.com",
            "/",
            &[],
            &mut buffer,
        ));

        assert!(matches!(
            result,
            Err(Error::InvalidResponse("Incomplete response body"))
        ));
    }

    #[test]
    fn test_io_client_close_delimited_response() {
        // No Content-Length and not chunked: connection close delimits the body.
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nhello";
        let mut stream = MockStream::<128, 256>::new(raw);
        let client = HttpClient::new();
        let mut buffer = [0; 64];

        let (response, total) = futures_lite::future::block_on(client.get(
            &mut stream,
            "example.com",
            "/",
            &[],
            &mut buffer,
        ))
        .unwrap();

        assert_eq!(total, raw.len());
        assert_eq!(response.body.as_str(), Some("hello"));
    }

    #[test]
    fn test_io_client_empty_response_buffer() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
        let mut stream = MockStream::<128, 256>::new(response);
        let client = HttpClient::new();
        let mut buffer = [0; 0];

        let result = futures_lite::future::block_on(client.get(
            &mut stream,
            "example.com",
            "/",
            &[],
            &mut buffer,
        ));

        assert!(matches!(result, Err(Error::NoResponse)));
    }

    #[test]
    fn test_parse_endpoint_defaults() {
        let endpoint = parse_endpoint("http://example.com").unwrap();
        assert_eq!(endpoint.scheme, "http");
        assert_eq!(endpoint.host, "example.com");
        assert_eq!(endpoint.port, DEFAULT_HTTP_PORT);
        assert_eq!(endpoint.path, "/");

        let endpoint = parse_endpoint("https://example.com").unwrap();
        assert_eq!(endpoint.scheme, "https");
        assert_eq!(endpoint.port, DEFAULT_HTTPS_PORT);
        assert_eq!(endpoint.path, "/");
    }

    #[test]
    fn test_parse_endpoint_explicit_port_and_path() {
        let endpoint = parse_endpoint("https://example.com:8443/api?x=1").unwrap();
        assert_eq!(endpoint.scheme, "https");
        assert_eq!(endpoint.host, "example.com");
        assert_eq!(endpoint.port, 8443);
        assert_eq!(endpoint.path, "/api?x=1");
    }

    #[test]
    fn test_parse_endpoint_ipv6() {
        let endpoint = parse_endpoint("http://[::1]:8080/x").unwrap();
        assert_eq!(endpoint.host, "[::1]");
        assert_eq!(endpoint.port, 8080);
        assert_eq!(endpoint.path, "/x");

        let endpoint = parse_endpoint("http://[::1]").unwrap();
        assert_eq!(endpoint.host, "[::1]");
        assert_eq!(endpoint.port, DEFAULT_HTTP_PORT);
    }

    #[test]
    fn test_parse_endpoint_invalid() {
        assert!(matches!(
            parse_endpoint("ftp://example.com"),
            Err(Error::InvalidUrl)
        ));
        assert!(matches!(
            parse_endpoint("example.com"),
            Err(Error::InvalidUrl)
        ));
    }

    #[derive(Debug)]
    struct FlakyError;

    impl core::fmt::Display for FlakyError {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("transient read failure")
        }
    }

    impl core::error::Error for FlakyError {}

    impl embedded_io_async::Error for FlakyError {
        fn kind(&self) -> embedded_io_async::ErrorKind {
            embedded_io_async::ErrorKind::Other
        }
    }

    /// Stream whose first `failing_reads` reads fail, then serves `input`.
    struct FlakyStream<const IN: usize, const OUT: usize> {
        inner: MockStream<IN, OUT>,
        failing_reads: usize,
    }

    impl<const IN: usize, const OUT: usize> FlakyStream<IN, OUT> {
        fn new(failing_reads: usize, input: &[u8]) -> Self {
            Self {
                inner: MockStream::new(input),
                failing_reads,
            }
        }
    }

    impl<const IN: usize, const OUT: usize> ErrorType for FlakyStream<IN, OUT> {
        type Error = FlakyError;
    }

    impl<const IN: usize, const OUT: usize> Read for FlakyStream<IN, OUT> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            if self.failing_reads > 0 {
                self.failing_reads -= 1;
                return Err(FlakyError);
            }
            self.inner
                .read(buf)
                .await
                .map_err(|infallible| match infallible {})
        }
    }

    impl<const IN: usize, const OUT: usize> Write for FlakyStream<IN, OUT> {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            self.inner
                .write(buf)
                .await
                .map_err(|infallible| match infallible {})
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            self.inner
                .flush()
                .await
                .map_err(|infallible| match infallible {})
        }
    }

    #[test]
    fn test_io_client_retries_transient_read_errors_with_delay() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
        let mut stream = FlakyStream::<128, 256>::new(2, response);
        let client = DefaultHttpClient::with_options(HttpClientOptions {
            max_retries: 5,
            ..HttpClientOptions::default()
        });
        let mut buffer = [0; 64];
        let delays = core::cell::Cell::new(0usize);

        let (response, _) = futures_lite::future::block_on(client.request_with_retry_delay(
            &mut stream,
            HttpClientRequest {
                method: HttpMethod::GET,
                host: "example.com",
                path: "/",
                headers: &[],
                body: None,
            },
            &mut buffer,
            || {
                delays.set(delays.get() + 1);
                core::future::ready(())
            },
        ))
        .unwrap();

        assert_eq!(response.status_code, StatusCode::Ok);
        assert_eq!(delays.get(), 2);
    }

    #[test]
    fn test_io_client_retry_exhaustion() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
        let mut stream = FlakyStream::<128, 256>::new(usize::MAX, response);
        let client = DefaultHttpClient::with_options(HttpClientOptions {
            max_retries: 2,
            ..HttpClientOptions::default()
        });
        let mut buffer = [0; 64];
        let delays = core::cell::Cell::new(0usize);

        let result = futures_lite::future::block_on(client.request_with_retry_delay(
            &mut stream,
            HttpClientRequest {
                method: HttpMethod::GET,
                host: "example.com",
                path: "/",
                headers: &[],
                body: None,
            },
            &mut buffer,
            || {
                delays.set(delays.get() + 1);
                core::future::ready(())
            },
        ));

        assert!(matches!(result, Err(Error::TcpError)));
        // max_retries counts total attempts: 2 failures, 1 delay in between.
        assert_eq!(delays.get(), 1);
    }
}
