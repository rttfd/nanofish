use crate::{
    codec,
    error::Error,
    header::HttpHeader,
    method::HttpMethod,
    options::HttpClientOptions,
    protocol::{DEFAULT_HTTP_PORT, DEFAULT_HTTPS_PORT},
    response::HttpResponse,
};
use core::fmt::Write as _;
use core::future::Future;
use embedded_io_async::{Read, Write};
use heapless::String;

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

impl HttpEndpoint<'_> {
    /// Build the value for the `Host` header of a request to this endpoint.
    ///
    /// This is the host, followed by `:port` unless the port is the default
    /// for the scheme (80 for `http`, 443 for `https`). A request to
    /// `http://example.com:8080/` must send `Host: example.com:8080`, otherwise
    /// virtual-host routing and generated redirects point at the wrong port.
    ///
    /// # Errors
    ///
    /// Returns [`Error::BufferOverflow`] if the value does not fit in `N` bytes.
    pub fn host_header<const N: usize>(&self) -> Result<String<N>, Error> {
        let mut value = String::new();
        value
            .push_str(self.host)
            .map_err(|_| Error::BufferOverflow)?;
        if self.port != default_port(self.scheme) {
            write!(value, ":{}", self.port).map_err(|_| Error::BufferOverflow)?;
        }
        Ok(value)
    }
}

/// Default port for a URL scheme.
fn default_port(scheme: &str) -> u16 {
    if scheme == "https" {
        DEFAULT_HTTPS_PORT
    } else {
        DEFAULT_HTTP_PORT
    }
}

/// Parse an HTTP or HTTPS URL into endpoint metadata.
///
/// The URL fragment (`#...`) is dropped, since it is never sent to the server.
/// IPv6 hosts keep their brackets, as in `[::1]`. A port is optional; an empty
/// one (`host:`) means the default for the scheme.
///
/// # Errors
///
/// Returns [`Error::InvalidUrl`] if the endpoint:
///
/// - does not start with `http://` or `https://`,
/// - has an empty host or a malformed IPv6 host,
/// - has a port that is not in `1..=65535`,
/// - contains user information (`user@host`), which is not supported, or
/// - has a query but no path (`http://host?x=1`). Write `http://host/?x=1`
///   instead: the query cannot be borrowed into a request target that starts
///   with `/` without a buffer.
pub fn parse_endpoint(endpoint: &str) -> Result<HttpEndpoint<'_>, Error> {
    let (scheme, rest) = if let Some(rest) = endpoint.strip_prefix("http://") {
        ("http", rest)
    } else if let Some(rest) = endpoint.strip_prefix("https://") {
        ("https", rest)
    } else {
        return Err(Error::InvalidUrl);
    };

    // The fragment is client-side only and must not reach the request target.
    let rest = rest.split_once('#').map_or(rest, |(before, _)| before);
    let (authority, path) = rest.split_at(rest.find(['/', '?']).unwrap_or(rest.len()));
    if path.starts_with('?') || authority.contains('@') {
        return Err(Error::InvalidUrl);
    }
    let path = if path.is_empty() { "/" } else { path };

    let (host, port) = split_host_port(authority, default_port(scheme))?;

    Ok(HttpEndpoint {
        scheme,
        host,
        port,
        path,
    })
}

/// Split a URL authority into host and port, using `default_port` when the
/// port is absent or empty.
fn split_host_port(authority: &str, default_port: u16) -> Result<(&str, u16), Error> {
    let (host, port) = if authority.starts_with('[') {
        let close = authority.find(']').ok_or(Error::InvalidUrl)?;
        let (host, rest) = authority.split_at(close + 1);
        if host == "[]" {
            return Err(Error::InvalidUrl);
        }
        let port = if rest.is_empty() {
            None
        } else {
            Some(rest.strip_prefix(':').ok_or(Error::InvalidUrl)?)
        };
        (host, port)
    } else {
        // An unbracketed host with more than one `:` ends up with a port
        // that does not parse, which rejects bare IPv6 addresses.
        authority
            .split_once(':')
            .map_or((authority, None), |(host, port)| (host, Some(port)))
    };

    if host.is_empty() {
        return Err(Error::InvalidUrl);
    }
    let port = match port.filter(|port| !port.is_empty()) {
        None => default_port,
        Some(port) => port
            .parse::<u16>()
            .ok()
            .filter(|&port| port != 0)
            .ok_or(Error::InvalidUrl)?,
    };
    Ok((host, port))
}

/// Request metadata for [`HttpClient`].
pub struct HttpClientRequest<'a> {
    /// HTTP method to send.
    pub method: HttpMethod,
    /// Host value used for the HTTP `Host` header.
    ///
    /// Include the port when it is not the default for the scheme; see
    /// [`HttpEndpoint::host_header`].
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
            request.method,
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
/// a body without either is delimited by connection close. Responses to `HEAD`
/// requests and `1xx`, `204` and `304` responses have no body and are complete
/// at the end of the headers.
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
    method: HttpMethod,
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
                if codec::response_complete(method, &response_buffer[..total_read]) {
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

    if codec::response_complete(method, &response_buffer[..total_read]) {
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
    fn test_io_client_head_response_without_body() {
        // HEAD advertises the body length but never sends the body.
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 100\r\n\r\n";
        let mut stream = MockStream::<128, 256>::new(raw);
        let client = HttpClient::new();
        let mut buffer = [0; 128];

        let (response, total) = futures_lite::future::block_on(client.request(
            &mut stream,
            HttpClientRequest {
                method: HttpMethod::HEAD,
                host: "example.com",
                path: "/",
                headers: &[],
                body: None,
            },
            &mut buffer,
        ))
        .unwrap();

        assert_eq!(total, raw.len());
        assert_eq!(response.status_code, StatusCode::Ok);
        assert_eq!(response.content_length(), Some(100));
        assert_eq!(response.body, ResponseBody::Empty);
    }

    #[test]
    fn test_io_client_head_response_over_get_is_incomplete() {
        // The same bytes are a truncated response to a GET.
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n";
        let mut stream = MockStream::<128, 256>::new(raw);
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
    fn test_io_client_not_modified_with_content_length() {
        let raw = b"HTTP/1.1 304 Not Modified\r\nContent-Length: 5\r\nETag: \"x\"\r\n\r\n";
        let mut stream = MockStream::<128, 256>::new(raw);
        let client = HttpClient::new();
        let mut buffer = [0; 128];

        let (response, _) = futures_lite::future::block_on(client.get(
            &mut stream,
            "example.com",
            "/",
            &[],
            &mut buffer,
        ))
        .unwrap();

        assert_eq!(response.status_code, StatusCode::NotModified);
        assert_eq!(response.body, ResponseBody::Empty);
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
    fn test_host_header_omits_default_port() {
        let endpoint = parse_endpoint("http://example.com/x").unwrap();
        assert_eq!(
            endpoint.host_header::<64>().unwrap().as_str(),
            "example.com"
        );

        let endpoint = parse_endpoint("https://example.com:443/x").unwrap();
        assert_eq!(
            endpoint.host_header::<64>().unwrap().as_str(),
            "example.com"
        );

        // An explicit default port is the same as no port.
        let endpoint = parse_endpoint("http://example.com:80").unwrap();
        assert_eq!(
            endpoint.host_header::<64>().unwrap().as_str(),
            "example.com"
        );
    }

    #[test]
    fn test_host_header_includes_non_default_port() {
        let endpoint = parse_endpoint("http://example.com:8080/x").unwrap();
        assert_eq!(
            endpoint.host_header::<64>().unwrap().as_str(),
            "example.com:8080"
        );

        // The default port of one scheme is not the default of the other.
        let endpoint = parse_endpoint("https://example.com:80").unwrap();
        assert_eq!(
            endpoint.host_header::<64>().unwrap().as_str(),
            "example.com:80"
        );
        let endpoint = parse_endpoint("http://example.com:443").unwrap();
        assert_eq!(
            endpoint.host_header::<64>().unwrap().as_str(),
            "example.com:443"
        );

        let endpoint = parse_endpoint("http://[::1]:8080/x").unwrap();
        assert_eq!(endpoint.host_header::<64>().unwrap().as_str(), "[::1]:8080");
    }

    #[test]
    fn test_host_header_overflow() {
        let endpoint = parse_endpoint("http://example.com:8080").unwrap();
        // "example.com" fits, ":8080" does not.
        assert!(matches!(
            endpoint.host_header::<13>(),
            Err(Error::BufferOverflow)
        ));
        assert!(matches!(
            endpoint.host_header::<5>(),
            Err(Error::BufferOverflow)
        ));
        assert!(endpoint.host_header::<16>().is_ok());
    }

    #[test]
    fn test_io_client_sends_host_header_with_port() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
        let mut stream = MockStream::<128, 256>::new(response);
        let client = HttpClient::new();
        let mut buffer = [0; 64];
        let endpoint = parse_endpoint("http://example.com:8080/status").unwrap();
        let host = endpoint.host_header::<64>().unwrap();

        futures_lite::future::block_on(client.get(
            &mut stream,
            &host,
            endpoint.path,
            &[],
            &mut buffer,
        ))
        .unwrap();

        let request = core::str::from_utf8(&stream.output).unwrap();
        assert!(request.starts_with("GET /status HTTP/1.1\r\nHost: example.com:8080\r\n"));
    }

    #[test]
    fn test_parse_endpoint_strips_fragment() {
        let endpoint = parse_endpoint("http://example.com/a?x=1#top").unwrap();
        assert_eq!(endpoint.host, "example.com");
        assert_eq!(endpoint.path, "/a?x=1");

        let endpoint = parse_endpoint("http://example.com#top").unwrap();
        assert_eq!(endpoint.host, "example.com");
        assert_eq!(endpoint.path, "/");

        let endpoint = parse_endpoint("http://example.com:8080#top/not-a-path").unwrap();
        assert_eq!(endpoint.port, 8080);
        assert_eq!(endpoint.path, "/");
    }

    #[test]
    fn test_parse_endpoint_query_without_path_is_rejected() {
        // This used to put `?x=1` into the host.
        for url in ["http://example.com?x=1", "http://example.com:8080?x=1#f"] {
            assert!(
                matches!(parse_endpoint(url), Err(Error::InvalidUrl)),
                "{url}"
            );
        }
        let endpoint = parse_endpoint("http://example.com/?x=1").unwrap();
        assert_eq!(endpoint.path, "/?x=1");
    }

    #[test]
    fn test_parse_endpoint_invalid_port() {
        for url in [
            "http://example.com:abc",
            "http://example.com:99999",
            "http://example.com:0",
            "http://example.com:80:81",
            "http://example.com:-1/x",
            "http://[::1]:abc",
        ] {
            assert!(
                matches!(parse_endpoint(url), Err(Error::InvalidUrl)),
                "{url}"
            );
        }

        // An empty port means the default port.
        let endpoint = parse_endpoint("http://example.com:/x").unwrap();
        assert_eq!(endpoint.host, "example.com");
        assert_eq!(endpoint.port, DEFAULT_HTTP_PORT);
        let endpoint = parse_endpoint("https://[::1]:").unwrap();
        assert_eq!(endpoint.host, "[::1]");
        assert_eq!(endpoint.port, DEFAULT_HTTPS_PORT);

        let endpoint = parse_endpoint("http://example.com:65535").unwrap();
        assert_eq!(endpoint.port, 65535);
    }

    #[test]
    fn test_parse_endpoint_invalid_host() {
        for url in [
            "http://",
            "http:///path",
            "http://:80",
            "http://?x=1",
            "http://#f",
            "http://[]",
            "http://[::1",
            "http://[::1]x",
            "http://[::1]:80:81",
            // A bare IPv6 address must be bracketed.
            "http://::1/",
        ] {
            assert!(
                matches!(parse_endpoint(url), Err(Error::InvalidUrl)),
                "{url}"
            );
        }
    }

    #[test]
    fn test_parse_endpoint_user_info_is_rejected() {
        for url in [
            "http://user@example.com/",
            "http://user:secret@example.com:8080/x",
        ] {
            assert!(
                matches!(parse_endpoint(url), Err(Error::InvalidUrl)),
                "{url}"
            );
        }
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
