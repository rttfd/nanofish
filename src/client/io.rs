use crate::{
    client::{
        builder::RequestBuilder,
        stream::{BodySink, ChunkDecoder, StreamedResponse},
    },
    codec::{self, BodyFraming},
    error::Error,
    header::HttpHeader,
    method::HttpMethod,
    options::HttpClientOptions,
    protocol::{self, DEFAULT_HTTP_PORT, DEFAULT_HTTPS_PORT, DOUBLE_CRLF_LEN},
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
    /// Request path without the query string. Always starts with `/`; a URL
    /// without a path yields `/`.
    pub path: &'a str,
    /// Query string without the leading `?`, if the URL has one. It is
    /// `Some("")` for a URL that ends in a bare `?`.
    pub query: Option<&'a str>,
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
/// The path and query are returned separately, so `http://host?x=1` yields the
/// path `/` and the query `x=1`. The URL fragment (`#...`) is dropped, since
/// it is never sent to the server. IPv6 hosts keep their brackets, as in
/// `[::1]`. A port is optional; an empty one (`host:`) means the default for
/// the scheme.
///
/// # Errors
///
/// Returns [`Error::InvalidUrl`] if the endpoint:
///
/// - does not start with `http://` or `https://`,
/// - has an empty host or a malformed IPv6 host,
/// - has a port that is not in `1..=65535`, or
/// - contains user information (`user@host`), which is not supported.
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
    let (authority, target) = rest.split_at(rest.find(['/', '?']).unwrap_or(rest.len()));
    if authority.contains('@') {
        return Err(Error::InvalidUrl);
    }
    let (path, query) = target
        .split_once('?')
        .map_or((target, None), |(path, query)| (path, Some(query)));
    let path = if path.is_empty() { "/" } else { path };

    let (host, port) = split_host_port(authority, default_port(scheme))?;

    Ok(HttpEndpoint {
        scheme,
        host,
        port,
        path,
        query,
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

/// A response together with whether its connection can be reused; see
/// [`HttpClient::exchange`].
#[derive(Debug)]
pub struct Exchange<'b> {
    /// The parsed response.
    pub response: HttpResponse<'b>,
    /// How many bytes of the response buffer hold the response.
    pub used: usize,
    /// Whether the stream may carry another request.
    pub reusable: bool,
}

/// Whether the response asks for the connection to be closed.
fn wants_close(response: &HttpResponse<'_>) -> bool {
    response
        .get_header(crate::header::headers::CONNECTION)
        .is_some_and(|value| {
            value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("close"))
        })
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
    /// Request path, for example `/` or `/api`. It may also carry the query
    /// (`/api?limit=10`) when `query` is `None`.
    pub path: &'a str,
    /// Query string without the leading `?`, sent after `path` when present.
    ///
    /// Together with `path` this is the request target; use
    /// [`HttpEndpoint::path`] and [`HttpEndpoint::query`] for a parsed URL.
    pub query: Option<&'a str>,
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
        let exchange = self
            .exchange_with_retry_delay(stream, request, response_buffer, false, retry_delay)
            .await?;
        Ok((exchange.response, exchange.used))
    }

    /// Send one request and report whether the connection can carry another.
    ///
    /// With `keep_alive` the request does not ask the server to close the
    /// connection, so an HTTP/1.1 server leaves it open. The returned
    /// [`Exchange::reusable`] says whether the stream may be used for the next
    /// request: it is `true` only if `keep_alive` was asked for, the response
    /// is HTTP/1.1, its end was delimited by `Content-Length`, chunked coding
    /// or the absence of a body (not by the connection closing), and it did not
    /// say `Connection: close`. When it is `false`, drop the stream and
    /// connect again.
    ///
    /// A server may also close an idle connection at any time; the next request
    /// on it then fails with [`Error::NoResponse`] or a [`Error::TcpError`],
    /// and can be retried on a new connection.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpClient::request`].
    pub async fn exchange<'b, S>(
        &self,
        stream: &mut S,
        request: HttpClientRequest<'_>,
        response_buffer: &'b mut [u8],
        keep_alive: bool,
    ) -> Result<Exchange<'b>, Error>
    where
        S: Read + Write,
    {
        self.exchange_with_retry_delay(stream, request, response_buffer, keep_alive, || {
            core::future::ready(())
        })
        .await
    }

    /// Like [`HttpClient::exchange`], sleeping via `retry_delay` between read
    /// retries.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpClient::request`].
    pub async fn exchange_with_retry_delay<'b, S, D, Fut>(
        &self,
        stream: &mut S,
        request: HttpClientRequest<'_>,
        response_buffer: &'b mut [u8],
        keep_alive: bool,
        retry_delay: D,
    ) -> Result<Exchange<'b>, Error>
    where
        S: Read + Write,
        D: Fn() -> Fut,
        Fut: Future<Output = ()>,
    {
        self.send_request(stream, &request, keep_alive).await?;

        let (total_read, framed) = read_response(
            stream,
            request.method,
            response_buffer,
            self.options.max_retries,
            retry_delay,
        )
        .await?;
        codec::check_response_framing(&response_buffer[..total_read])?;
        let http11 = response_buffer[..total_read].starts_with(b"HTTP/1.1 ");
        let total_read = codec::dechunk(response_buffer, total_read)?;
        let response = codec::parse_resp(&response_buffer[..total_read])?;
        let reusable = keep_alive && framed && http11 && !wants_close(&response);
        Ok(Exchange {
            response,
            used: total_read,
            reusable,
        })
    }

    /// Write a request (head and body) to the stream and flush it.
    async fn send_request<S>(
        &self,
        stream: &mut S,
        request: &HttpClientRequest<'_>,
        keep_alive: bool,
    ) -> Result<(), Error>
    where
        S: Write,
    {
        let http_request = codec::build_req_with::<RQ>(
            request.method,
            request.host,
            request.path,
            request.query,
            request.headers,
            request.body,
            keep_alive,
        )?;

        stream
            .write_all(http_request.as_bytes())
            .await
            .map_err(Error::io)?;
        if let Some(body_data) = request.body {
            stream.write_all(body_data).await.map_err(Error::io)?;
        }
        stream.flush().await.map_err(Error::io)
    }

    /// Send one HTTP request and hand the response body to `sink` piece by
    /// piece, instead of collecting it in a buffer.
    ///
    /// `buffer` only has to hold the status line and headers, plus whatever
    /// piece of the body is read at a time, so a response of any size passes
    /// through a small buffer. The sink first sees the head
    /// ([`BodySink::head`]) and can reject the response, then the decoded body
    /// ([`BodySink::chunk`]); a chunked body is delivered without its framing.
    ///
    /// Read errors are retried like in [`HttpClient::request`].
    ///
    /// # Errors
    ///
    /// - [`Error::BufferOverflow`] if the response head does not fit in
    ///   `buffer`.
    /// - [`Error::NoResponse`] if the connection closes before any response
    ///   byte arrives.
    /// - [`Error::InvalidResponse`] if the response is malformed or is cut
    ///   short, so a truncated download is never reported as complete.
    /// - Any error returned by the sink.
    /// - The errors of building and sending the request, see
    ///   [`HttpClient::request`].
    pub async fn request_streaming<S, K>(
        &self,
        stream: &mut S,
        request: HttpClientRequest<'_>,
        buffer: &mut [u8],
        sink: &mut K,
    ) -> Result<StreamedResponse, Error>
    where
        S: Read + Write,
        K: BodySink,
    {
        self.request_streaming_with_retry_delay(stream, request, buffer, sink, || {
            core::future::ready(())
        })
        .await
    }

    /// Like [`HttpClient::request_streaming`], sleeping via `retry_delay`
    /// between read retries.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpClient::request_streaming`].
    pub async fn request_streaming_with_retry_delay<S, K, D, Fut>(
        &self,
        stream: &mut S,
        request: HttpClientRequest<'_>,
        buffer: &mut [u8],
        sink: &mut K,
        mut retry_delay: D,
    ) -> Result<StreamedResponse, Error>
    where
        S: Read + Write,
        K: BodySink,
        D: Fn() -> Fut,
        Fut: Future<Output = ()>,
    {
        let method = request.method;
        self.send_request(stream, &request, false).await?;

        let mut retries = self.options.max_retries;
        let mut held = 0;
        let mut searched: usize = 0;
        let head_end = loop {
            if held == buffer.len() {
                return Err(Error::BufferOverflow);
            }
            let n =
                read_retrying(stream, &mut buffer[held..], &mut retries, &mut retry_delay).await?;
            if n == 0 {
                return Err(if held == 0 {
                    Error::NoResponse
                } else {
                    Error::InvalidResponse("Incomplete response head")
                });
            }
            held += n;
            let from = searched.saturating_sub(DOUBLE_CRLF_LEN - 1);
            if let Some(pos) = protocol::find_double_crlf(&buffer[from..held]) {
                break from + pos + DOUBLE_CRLF_LEN;
            }
            searched = held;
        };

        codec::check_response_framing(&buffer[..head_end])?;
        let framing = codec::response_framing(method, &buffer[..head_end]);
        let status_code = {
            let response = codec::parse_resp(&buffer[..head_end])?;
            sink.head(&response).await?;
            response.status_code
        };

        // Keep what followed the head in the same read: it is body.
        buffer.copy_within(head_end..held, 0);
        held -= head_end;

        let mut delivered = 0;
        match framing {
            BodyFraming::None => {}
            BodyFraming::Length(total) => {
                while delivered < total {
                    if held == 0 {
                        held = read_retrying(stream, &mut *buffer, &mut retries, &mut retry_delay)
                            .await?;
                        if held == 0 {
                            return Err(Error::InvalidResponse("Incomplete response body"));
                        }
                    }
                    // Bytes beyond the declared length are not part of the body.
                    let take = held.min(total - delivered);
                    sink.chunk(&buffer[..take]).await?;
                    delivered += take;
                    held = 0;
                }
            }
            BodyFraming::UntilClose => loop {
                if held > 0 {
                    sink.chunk(&buffer[..held]).await?;
                    delivered += held;
                }
                held = read_retrying(stream, &mut *buffer, &mut retries, &mut retry_delay).await?;
                if held == 0 {
                    break;
                }
            },
            BodyFraming::Chunked => {
                let mut decoder = ChunkDecoder::new();
                loop {
                    let mut pos = 0;
                    while pos < held && !decoder.is_done() {
                        let step = decoder.step(&buffer[pos..held])?;
                        if let Some(range) = step.data {
                            sink.chunk(&buffer[pos + range.start..pos + range.end])
                                .await?;
                            delivered += range.len();
                        }
                        pos += step.consumed;
                    }
                    if decoder.is_done() {
                        break;
                    }
                    held =
                        read_retrying(stream, &mut *buffer, &mut retries, &mut retry_delay).await?;
                    if held == 0 {
                        return Err(Error::InvalidResponse("Incomplete response body"));
                    }
                }
            }
        }

        Ok(StreamedResponse {
            status_code,
            body_len: delivered,
        })
    }

    /// Send a request described by a [`RequestBuilder`] over an already-connected
    /// stream.
    ///
    /// `path` may carry a query string (`/api?limit=10`).
    ///
    /// # Errors
    ///
    /// Returns [`Error::BufferOverflow`] if the builder was given too many
    /// headers, otherwise the same errors as [`HttpClient::request`].
    pub async fn send<'b, S>(
        &self,
        stream: &mut S,
        host: &str,
        path: &str,
        builder: &RequestBuilder<'_>,
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error>
    where
        S: Read + Write,
    {
        builder.check()?;
        self.request(
            stream,
            HttpClientRequest {
                method: builder.method(),
                host,
                path,
                query: None,
                headers: builder.headers(),
                body: builder.body_bytes(),
            },
            response_buffer,
        )
        .await
    }

    /// Convenience method for making a POST request with a JSON body over an
    /// already-connected stream. Adds `Content-Type: application/json`.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpClient::send`].
    pub async fn post_json<'b, S>(
        &self,
        stream: &mut S,
        host: &str,
        path: &str,
        headers: &[HttpHeader<'_>],
        json: &str,
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error>
    where
        S: Read + Write,
    {
        let builder = RequestBuilder::post().headers_from(headers).json(json);
        self.send(stream, host, path, &builder, response_buffer)
            .await
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
                query: None,
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
                query: None,
                headers,
                body: Some(body),
            },
            response_buffer,
        )
        .await
    }
}

/// One read from `stream`, retrying transient errors like [`read_response`]:
/// `retries` counts the read attempts left across the whole response.
async fn read_retrying<S, D, Fut>(
    stream: &mut S,
    buffer: &mut [u8],
    retries: &mut usize,
    retry_delay: &mut D,
) -> Result<usize, Error>
where
    S: Read,
    D: FnMut() -> Fut,
    Fut: Future<Output = ()>,
{
    loop {
        if *retries == 0 {
            return Err(Error::NoResponse);
        }
        match stream.read(buffer).await {
            Ok(n) => return Ok(n),
            Err(error) => {
                *retries -= 1;
                if *retries == 0 {
                    return Err(Error::io(error));
                }
                retry_delay().await;
            }
        }
    }
}

/// Read a full HTTP response into `buffer`, retrying transient read errors.
///
/// Returns the number of bytes read and whether the response end was
/// delimited by its framing (`true`) rather than by the connection closing.
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
/// - [`Error::TcpError`] if `max_retries` read attempts fail, with the kind of
///   the last failure.
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
) -> Result<(usize, bool), Error>
where
    S: Read,
    D: Fn() -> Fut,
    Fut: Future<Output = ()>,
{
    let mut total_read = 0;
    let mut retries = max_retries;
    let mut saw_eof = false;
    let mut scan = codec::ResponseScan::new(method);

    while total_read < response_buffer.len() && retries > 0 {
        match stream.read(&mut response_buffer[total_read..]).await {
            Ok(0) => {
                saw_eof = true;
                break;
            }
            Ok(n) => {
                total_read += n;
                if scan.is_complete(&response_buffer[..total_read]) {
                    return Ok((total_read, true));
                }
            }
            Err(error) => {
                retries -= 1;
                if retries == 0 {
                    return Err(Error::io(error));
                }
                retry_delay().await;
            }
        }
    }

    if total_read == 0 {
        return Err(Error::NoResponse);
    }

    if scan.is_complete(&response_buffer[..total_read]) {
        return Ok((total_read, true));
    }

    if saw_eof {
        if scan.declares_length() {
            return Err(Error::InvalidResponse("Incomplete response body"));
        }
        // No length was declared: connection close delimits the body.
        return Ok((total_read, false));
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
                query: None,
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
    fn test_io_client_rejects_conflicting_content_length() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Length: 6\r\n\r\nhello";
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
            Err(Error::InvalidResponse("Conflicting Content-Length"))
        ));
    }

    #[test]
    fn test_io_client_rejects_malformed_content_length() {
        for value in ["abc", "+5", "-1", ""] {
            let mut raw = heapless::Vec::<u8, 128>::new();
            raw.extend_from_slice(b"HTTP/1.1 200 OK\r\nContent-Length: ")
                .unwrap();
            raw.extend_from_slice(value.as_bytes()).unwrap();
            raw.extend_from_slice(b"\r\n\r\nhello").unwrap();
            let mut stream = MockStream::<128, 256>::new(&raw);
            let client = HttpClient::new();
            let mut buffer = [0; 128];

            let result = futures_lite::future::block_on(client.get(
                &mut stream,
                "example.com",
                "/",
                &[],
                &mut buffer,
            ));

            assert!(
                matches!(
                    result,
                    Err(Error::InvalidResponse("Invalid Content-Length"))
                ),
                "{value:?}"
            );
        }
    }

    #[test]
    fn test_io_client_post_without_body_sends_content_length_zero() {
        let response = b"HTTP/1.1 204 No Content\r\n\r\n";
        let mut stream = MockStream::<128, 256>::new(response);
        let client = HttpClient::new();
        let mut buffer = [0; 128];

        futures_lite::future::block_on(client.request(
            &mut stream,
            HttpClientRequest {
                method: HttpMethod::POST,
                host: "example.com",
                path: "/trigger",
                query: None,
                headers: &[],
                body: None,
            },
            &mut buffer,
        ))
        .unwrap();

        let request = core::str::from_utf8(&stream.output).unwrap();
        assert!(request.contains("Content-Length: 0\r\n"), "{request}");
    }

    #[test]
    fn test_io_client_send_with_request_builder() {
        let response = b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n";
        let mut stream = MockStream::<128, 512>::new(response);
        let client = HttpClient::new();
        let mut buffer = [0; 128];
        let token = crate::encode::bearer_auth::<32>("abc").unwrap();
        let builder = RequestBuilder::put()
            .authorization(&token)
            .header("X-Request-Id", "7")
            .json(r#"{"on":true}"#);

        let (response, _) = futures_lite::future::block_on(client.send(
            &mut stream,
            "example.com",
            "/led/3?fade=1",
            &builder,
            &mut buffer,
        ))
        .unwrap();

        assert_eq!(response.status_code, StatusCode::Created);
        let request = core::str::from_utf8(&stream.output).unwrap();
        assert!(request.starts_with("PUT /led/3?fade=1 HTTP/1.1\r\nHost: example.com\r\n"));
        assert!(request.contains("Authorization: Bearer abc\r\n"));
        assert!(request.contains("X-Request-Id: 7\r\n"));
        assert!(request.contains("Content-Type: application/json\r\n"));
        assert!(request.contains("Content-Length: 11\r\n"));
        assert!(request.ends_with(r#"{"on":true}"#));
    }

    #[test]
    fn test_io_client_send_rejects_too_many_headers() {
        let mut stream = MockStream::<128, 512>::new(b"");
        let client = HttpClient::new();
        let mut buffer = [0; 128];
        let mut builder = RequestBuilder::get();
        for _ in 0..=crate::protocol::MAX_HEADERS {
            builder = builder.header("X-A", "b");
        }

        let result = futures_lite::future::block_on(client.send(
            &mut stream,
            "example.com",
            "/",
            &builder,
            &mut buffer,
        ));

        assert!(matches!(result, Err(Error::BufferOverflow)));
        // Nothing was sent.
        assert!(stream.output.is_empty());
    }

    #[test]
    fn test_io_client_post_json() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
        let mut stream = MockStream::<128, 512>::new(response);
        let client = HttpClient::new();
        let mut buffer = [0; 128];

        futures_lite::future::block_on(client.post_json(
            &mut stream,
            "example.com",
            "/rpc",
            &[HttpHeader::new("X-Trace", "1")],
            r#"{"a":1}"#,
            &mut buffer,
        ))
        .unwrap();

        let request = core::str::from_utf8(&stream.output).unwrap();
        assert!(request.starts_with("POST /rpc HTTP/1.1\r\n"));
        assert!(request.contains("X-Trace: 1\r\n"));
        assert!(request.contains("Content-Type: application/json\r\n"));
        assert!(request.ends_with(r#"{"a":1}"#));
    }

    #[test]
    fn test_io_client_send_form_body() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
        let mut stream = MockStream::<128, 512>::new(response);
        let client = HttpClient::new();
        let mut buffer = [0; 128];
        let mut form = crate::encode::QueryBuilder::<64>::new();
        form.pair("name", "A B").unwrap().pair("k", "v&w").unwrap();
        let builder = RequestBuilder::post().form(form.as_bytes());

        futures_lite::future::block_on(client.send(
            &mut stream,
            "example.com",
            "/f",
            &builder,
            &mut buffer,
        ))
        .unwrap();

        let request = core::str::from_utf8(&stream.output).unwrap();
        assert!(request.contains("Content-Type: application/x-www-form-urlencoded\r\n"));
        assert!(request.ends_with("name=A%20B&k=v%26w"));
    }

    /// Collects what `request_streaming` hands to a sink.
    #[derive(Default)]
    struct Collect {
        status: Option<StatusCode>,
        content_length: Option<usize>,
        body: std::vec::Vec<u8>,
        chunk_sizes: std::vec::Vec<usize>,
        reject_status: Option<StatusCode>,
        fail_after: Option<usize>,
    }

    impl BodySink for Collect {
        async fn head(&mut self, response: &HttpResponse<'_>) -> Result<(), Error> {
            self.status = Some(response.status_code);
            self.content_length = response.content_length();
            if self.reject_status == Some(response.status_code) {
                return Err(Error::InvalidResponse("rejected by sink"));
            }
            Ok(())
        }

        async fn chunk(&mut self, data: &[u8]) -> Result<(), Error> {
            assert!(!data.is_empty());
            if self
                .fail_after
                .is_some_and(|limit| self.body.len() >= limit)
            {
                return Err(Error::BufferOverflow);
            }
            self.body.extend_from_slice(data);
            self.chunk_sizes.push(data.len());
            Ok(())
        }
    }

    /// Hands out at most `max` bytes per read.
    struct Trickle<const IN: usize, const OUT: usize> {
        inner: MockStream<IN, OUT>,
        max: usize,
    }

    impl<const IN: usize, const OUT: usize> ErrorType for Trickle<IN, OUT> {
        type Error = Infallible;
    }

    impl<const IN: usize, const OUT: usize> Read for Trickle<IN, OUT> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            let n = buf.len().min(self.max);
            self.inner.read(&mut buf[..n]).await
        }
    }

    impl<const IN: usize, const OUT: usize> Write for Trickle<IN, OUT> {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            self.inner.write(buf).await
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            self.inner.flush().await
        }
    }

    fn get_request(method: HttpMethod) -> HttpClientRequest<'static> {
        HttpClientRequest {
            method,
            host: "example.com",
            path: "/fw.bin",
            query: None,
            headers: &[],
            body: None,
        }
    }

    /// A response with a `len`-byte body of `0..=250` repeating.
    fn length_response(len: usize) -> std::vec::Vec<u8> {
        let mut raw = std::vec::Vec::from(
            std::format!("HTTP/1.1 200 OK\r\nContent-Length: {len}\r\n\r\n").as_bytes(),
        );
        raw.extend((0..len).map(|i| u8::try_from(i % 251).unwrap()));
        raw
    }

    #[test]
    fn test_streaming_length_body_through_small_buffer() {
        let raw = length_response(600);
        let mut stream = MockStream::<1024, 256>::new(&raw);
        let client = HttpClient::new();
        let mut buffer = [0; 100];
        let mut sink = Collect::default();

        let streamed = futures_lite::future::block_on(client.request_streaming(
            &mut stream,
            get_request(HttpMethod::GET),
            &mut buffer,
            &mut sink,
        ))
        .unwrap();

        assert_eq!(streamed.status_code, StatusCode::Ok);
        assert_eq!(streamed.body_len, 600);
        assert_eq!(sink.status, Some(StatusCode::Ok));
        assert_eq!(sink.content_length, Some(600));
        assert_eq!(sink.body, raw[raw.len() - 600..]);
        // The buffer is far smaller than the body.
        assert!(sink.chunk_sizes.iter().all(|&n| n <= 100));
        let request = core::str::from_utf8(&stream.output).unwrap();
        assert!(request.starts_with("GET /fw.bin HTTP/1.1\r\n"));
    }

    #[test]
    fn test_streaming_body_is_exact_for_every_read_size() {
        let raw = length_response(130);
        for max in [1, 2, 7, 50, 1000] {
            let mut stream = Trickle {
                inner: MockStream::<512, 256>::new(&raw),
                max,
            };
            let client = HttpClient::new();
            let mut buffer = [0; 64];
            let mut sink = Collect::default();

            let streamed = futures_lite::future::block_on(client.request_streaming(
                &mut stream,
                get_request(HttpMethod::GET),
                &mut buffer,
                &mut sink,
            ))
            .unwrap();

            assert_eq!(streamed.body_len, 130, "max {max}");
            assert_eq!(sink.body, raw[raw.len() - 130..], "max {max}");
        }
    }

    #[test]
    fn test_streaming_chunked_body_is_decoded() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n7\r\nab0\r\n\r\n\r\na;x=1\r\n0123456789\r\n0\r\nT: v\r\n\r\n";
        for max in [1, 3, 8, 1000] {
            let mut stream = Trickle {
                inner: MockStream::<256, 256>::new(raw),
                max,
            };
            let client = HttpClient::new();
            let mut buffer = [0; 48];
            let mut sink = Collect::default();

            let streamed = futures_lite::future::block_on(client.request_streaming(
                &mut stream,
                get_request(HttpMethod::GET),
                &mut buffer,
                &mut sink,
            ))
            .unwrap();

            assert_eq!(sink.body, b"ab0\r\n\r\n0123456789", "max {max}");
            assert_eq!(streamed.body_len, 17);
        }
    }

    #[test]
    fn test_streaming_close_delimited_body() {
        let raw = b"HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\n\r\nuntil the connection closes";
        let mut stream = Trickle {
            inner: MockStream::<256, 256>::new(raw),
            max: 5,
        };
        let client = HttpClient::new();
        let mut buffer = [0; 64];
        let mut sink = Collect::default();

        let streamed = futures_lite::future::block_on(client.request_streaming(
            &mut stream,
            get_request(HttpMethod::GET),
            &mut buffer,
            &mut sink,
        ))
        .unwrap();

        assert_eq!(sink.body, b"until the connection closes");
        assert_eq!(streamed.body_len, 27);
    }

    #[test]
    fn test_streaming_head_response_has_no_body() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 5000\r\n\r\n";
        let mut stream = MockStream::<128, 256>::new(raw);
        let client = HttpClient::new();
        let mut buffer = [0; 128];
        let mut sink = Collect::default();

        let streamed = futures_lite::future::block_on(client.request_streaming(
            &mut stream,
            get_request(HttpMethod::HEAD),
            &mut buffer,
            &mut sink,
        ))
        .unwrap();

        assert_eq!(streamed.body_len, 0);
        assert_eq!(sink.content_length, Some(5000));
        assert!(sink.chunk_sizes.is_empty());
    }

    #[test]
    fn test_streaming_empty_body_calls_no_chunks() {
        let raw = b"HTTP/1.1 204 No Content\r\n\r\n";
        let mut stream = MockStream::<64, 256>::new(raw);
        let client = HttpClient::new();
        let mut buffer = [0; 64];
        let mut sink = Collect::default();

        let streamed = futures_lite::future::block_on(client.request_streaming(
            &mut stream,
            get_request(HttpMethod::GET),
            &mut buffer,
            &mut sink,
        ))
        .unwrap();

        assert_eq!(streamed.status_code, StatusCode::NoContent);
        assert!(sink.chunk_sizes.is_empty());
    }

    #[test]
    fn test_streaming_sink_can_reject_the_response_before_the_body() {
        let raw = b"HTTP/1.1 404 Not Found\r\nContent-Length: 4\r\n\r\nnope";
        let mut stream = MockStream::<128, 256>::new(raw);
        let client = HttpClient::new();
        let mut buffer = [0; 128];
        let mut sink = Collect {
            reject_status: Some(StatusCode::NotFound),
            ..Collect::default()
        };

        let result = futures_lite::future::block_on(client.request_streaming(
            &mut stream,
            get_request(HttpMethod::GET),
            &mut buffer,
            &mut sink,
        ));

        assert!(matches!(
            result,
            Err(Error::InvalidResponse("rejected by sink"))
        ));
        assert!(sink.chunk_sizes.is_empty());
    }

    #[test]
    fn test_streaming_sink_error_stops_the_download() {
        let raw = length_response(300);
        let mut stream = MockStream::<512, 256>::new(&raw);
        let client = HttpClient::new();
        let mut buffer = [0; 80];
        let mut sink = Collect {
            fail_after: Some(100),
            ..Collect::default()
        };

        let result = futures_lite::future::block_on(client.request_streaming(
            &mut stream,
            get_request(HttpMethod::GET),
            &mut buffer,
            &mut sink,
        ));

        assert!(matches!(result, Err(Error::BufferOverflow)));
        assert!(sink.body.len() < 300);
    }

    #[test]
    fn test_streaming_truncated_body_is_an_error() {
        let mut raw = length_response(100);
        raw.truncate(raw.len() - 10);
        let mut stream = MockStream::<256, 256>::new(&raw);
        let client = HttpClient::new();
        let mut buffer = [0; 64];
        let mut sink = Collect::default();

        let result = futures_lite::future::block_on(client.request_streaming(
            &mut stream,
            get_request(HttpMethod::GET),
            &mut buffer,
            &mut sink,
        ));

        assert!(matches!(
            result,
            Err(Error::InvalidResponse("Incomplete response body"))
        ));

        // The same for an unfinished chunked body.
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n";
        let mut stream = MockStream::<128, 256>::new(raw);
        let result = futures_lite::future::block_on(client.request_streaming(
            &mut stream,
            get_request(HttpMethod::GET),
            &mut buffer,
            &mut Collect::default(),
        ));
        assert!(matches!(
            result,
            Err(Error::InvalidResponse("Incomplete response body"))
        ));
    }

    #[test]
    fn test_streaming_head_larger_than_buffer() {
        let raw = length_response(10);
        let mut stream = MockStream::<256, 256>::new(&raw);
        let client = HttpClient::new();
        let mut buffer = [0; 20];
        let mut sink = Collect::default();

        let result = futures_lite::future::block_on(client.request_streaming(
            &mut stream,
            get_request(HttpMethod::GET),
            &mut buffer,
            &mut sink,
        ));

        assert!(matches!(result, Err(Error::BufferOverflow)));
        assert_eq!(sink.status, None);
    }

    #[test]
    fn test_streaming_no_response() {
        let mut stream = MockStream::<8, 256>::new(b"");
        let client = HttpClient::new();
        let mut buffer = [0; 64];

        let result = futures_lite::future::block_on(client.request_streaming(
            &mut stream,
            get_request(HttpMethod::GET),
            &mut buffer,
            &mut Collect::default(),
        ));

        assert!(matches!(result, Err(Error::NoResponse)));
    }

    #[test]
    fn test_streaming_rejects_conflicting_content_length() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nContent-Length: 5\r\n\r\nabcd";
        let mut stream = MockStream::<128, 256>::new(raw);
        let client = HttpClient::new();
        let mut buffer = [0; 128];
        let mut sink = Collect::default();

        let result = futures_lite::future::block_on(client.request_streaming(
            &mut stream,
            get_request(HttpMethod::GET),
            &mut buffer,
            &mut sink,
        ));

        assert!(matches!(
            result,
            Err(Error::InvalidResponse("Conflicting Content-Length"))
        ));
        assert_eq!(sink.status, None);
    }

    /// A server that answers each flushed request with the next canned reply,
    /// the way a keep-alive connection behaves.
    struct ReplyStream<'r> {
        replies: &'r [&'r [u8]],
        next: usize,
        readable: std::vec::Vec<u8>,
        read_pos: usize,
        requests: std::vec::Vec<std::string::String>,
        current: std::vec::Vec<u8>,
    }

    impl<'r> ReplyStream<'r> {
        fn new(replies: &'r [&'r [u8]]) -> Self {
            Self {
                replies,
                next: 0,
                readable: std::vec::Vec::new(),
                read_pos: 0,
                requests: std::vec::Vec::new(),
                current: std::vec::Vec::new(),
            }
        }
    }

    impl ErrorType for ReplyStream<'_> {
        type Error = Infallible;
    }

    impl Read for ReplyStream<'_> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            let available = &self.readable[self.read_pos..];
            let n = buf.len().min(available.len());
            buf[..n].copy_from_slice(&available[..n]);
            self.read_pos += n;
            Ok(n)
        }
    }

    impl Write for ReplyStream<'_> {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            self.current.extend_from_slice(buf);
            Ok(buf.len())
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            if !self.current.is_empty() {
                self.requests
                    .push(std::string::String::from_utf8_lossy(&self.current).into_owned());
                self.current.clear();
                if let Some(reply) = self.replies.get(self.next) {
                    self.readable.extend_from_slice(reply);
                    self.next += 1;
                }
            }
            Ok(())
        }
    }

    fn keep_alive_request(path: &'static str) -> HttpClientRequest<'static> {
        HttpClientRequest {
            method: HttpMethod::GET,
            host: "example.com",
            path,
            query: None,
            headers: &[],
            body: None,
        }
    }

    #[test]
    fn test_exchange_reuses_a_keep_alive_connection() {
        let replies: [&[u8]; 3] = [
            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\none",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\ntwo\r\n0\r\n\r\n",
            b"HTTP/1.1 204 No Content\r\n\r\n",
        ];
        let mut stream = ReplyStream::new(&replies);
        let client = HttpClient::new();
        let mut buffer = [0; 128];

        for (path, body) in [("/1", Some("one")), ("/2", Some("two")), ("/3", None)] {
            let exchange = futures_lite::future::block_on(client.exchange(
                &mut stream,
                keep_alive_request(path),
                &mut buffer,
                true,
            ))
            .unwrap();
            assert!(exchange.reusable, "{path}");
            assert_eq!(exchange.response.body.as_str(), body, "{path}");
        }

        assert_eq!(stream.requests.len(), 3);
        for (request, path) in stream.requests.iter().zip(["/1", "/2", "/3"]) {
            assert!(request.starts_with(&std::format!("GET {path} HTTP/1.1\r\n")));
            assert!(!request.contains("Connection:"), "{request}");
            assert!(request.ends_with("\r\n\r\n"));
        }
    }

    #[test]
    fn test_exchange_without_keep_alive_asks_to_close() {
        let replies: [&[u8]; 1] = [b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok"];
        let mut stream = ReplyStream::new(&replies);
        let client = HttpClient::new();
        let mut buffer = [0; 128];

        let exchange = futures_lite::future::block_on(client.exchange(
            &mut stream,
            keep_alive_request("/"),
            &mut buffer,
            false,
        ))
        .unwrap();

        assert!(!exchange.reusable);
        assert!(stream.requests[0].contains("Connection: close\r\n"));
    }

    #[test]
    fn test_exchange_not_reusable_when_server_closes_or_cannot_persist() {
        let cases: [&[u8]; 6] = [
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nconnection: Keep-Alive, CLOSE\r\n\r\nok",
            b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\n\r\nok",
            // The body ends when the connection closes: nothing more can follow.
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nuntil close",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n0\r\n\r\n",
            b"HTTP/1.0 200 OK\r\nConnection: keep-alive\r\nContent-Length: 0\r\n\r\n",
        ];
        for reply in cases {
            let replies = [reply];
            let mut stream = ReplyStream::new(&replies);
            let client = HttpClient::new();
            let mut buffer = [0; 128];

            let exchange = futures_lite::future::block_on(client.exchange(
                &mut stream,
                keep_alive_request("/"),
                &mut buffer,
                true,
            ))
            .unwrap();

            assert!(
                !exchange.reusable,
                "{}",
                core::str::from_utf8(reply).unwrap()
            );
        }
    }

    #[test]
    fn test_exchange_head_response_is_reusable() {
        let replies: [&[u8]; 1] = [b"HTTP/1.1 200 OK\r\nContent-Length: 500\r\n\r\n"];
        let mut stream = ReplyStream::new(&replies);
        let client = HttpClient::new();
        let mut buffer = [0; 128];

        let exchange = futures_lite::future::block_on(client.exchange(
            &mut stream,
            HttpClientRequest {
                method: HttpMethod::HEAD,
                ..keep_alive_request("/")
            },
            &mut buffer,
            true,
        ))
        .unwrap();

        assert!(exchange.reusable);
        assert_eq!(exchange.response.content_length(), Some(500));
    }

    #[test]
    fn test_request_still_closes_the_connection() {
        let replies: [&[u8]; 1] = [b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok"];
        let mut stream = ReplyStream::new(&replies);
        let client = HttpClient::new();
        let mut buffer = [0; 128];

        futures_lite::future::block_on(client.get(
            &mut stream,
            "example.com",
            "/",
            &[],
            &mut buffer,
        ))
        .unwrap();

        assert!(stream.requests[0].contains("Connection: close\r\n"));
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
        assert_eq!(endpoint.path, "/api");
        assert_eq!(endpoint.query, Some("x=1"));
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
        assert_eq!(endpoint.path, "/a");
        assert_eq!(endpoint.query, Some("x=1"));

        let endpoint = parse_endpoint("http://example.com#top").unwrap();
        assert_eq!(endpoint.host, "example.com");
        assert_eq!(endpoint.path, "/");

        let endpoint = parse_endpoint("http://example.com:8080#top/not-a-path").unwrap();
        assert_eq!(endpoint.port, 8080);
        assert_eq!(endpoint.path, "/");
    }

    #[test]
    fn test_parse_endpoint_splits_path_and_query() {
        // A query without a path used to end up in the host name.
        let endpoint = parse_endpoint("http://example.com?x=1").unwrap();
        assert_eq!(endpoint.host, "example.com");
        assert_eq!(endpoint.path, "/");
        assert_eq!(endpoint.query, Some("x=1"));

        let endpoint = parse_endpoint("http://example.com:8080?x=1#f").unwrap();
        assert_eq!(endpoint.host, "example.com");
        assert_eq!(endpoint.port, 8080);
        assert_eq!(endpoint.path, "/");
        assert_eq!(endpoint.query, Some("x=1"));

        let endpoint = parse_endpoint("http://example.com/?x=1&y=2").unwrap();
        assert_eq!(endpoint.path, "/");
        assert_eq!(endpoint.query, Some("x=1&y=2"));

        let endpoint = parse_endpoint("http://[::1]:8080/a/b?q=a?b").unwrap();
        assert_eq!(endpoint.path, "/a/b");
        assert_eq!(endpoint.query, Some("q=a?b"));
    }

    #[test]
    fn test_parse_endpoint_without_query() {
        let endpoint = parse_endpoint("http://example.com").unwrap();
        assert_eq!(endpoint.query, None);

        let endpoint = parse_endpoint("http://example.com/a/b").unwrap();
        assert_eq!(endpoint.path, "/a/b");
        assert_eq!(endpoint.query, None);

        // A bare `?` is an empty query, kept so the request target round-trips.
        let endpoint = parse_endpoint("http://example.com/a?").unwrap();
        assert_eq!(endpoint.path, "/a");
        assert_eq!(endpoint.query, Some(""));
    }

    #[test]
    fn test_io_client_sends_endpoint_path_and_query() {
        for (url, target) in [
            ("http://example.com?x=1", "GET /?x=1 HTTP/1.1\r\n"),
            ("http://example.com/", "GET / HTTP/1.1\r\n"),
            (
                "http://example.com/a/b?x=1&y=2",
                "GET /a/b?x=1&y=2 HTTP/1.1\r\n",
            ),
            ("http://example.com/a?", "GET /a? HTTP/1.1\r\n"),
        ] {
            let response = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
            let mut stream = MockStream::<128, 256>::new(response);
            let client = HttpClient::new();
            let mut buffer = [0; 64];
            let endpoint = parse_endpoint(url).unwrap();

            futures_lite::future::block_on(client.request(
                &mut stream,
                HttpClientRequest {
                    method: HttpMethod::GET,
                    host: endpoint.host,
                    path: endpoint.path,
                    query: endpoint.query,
                    headers: &[],
                    body: None,
                },
                &mut buffer,
            ))
            .unwrap();

            let request = core::str::from_utf8(&stream.output).unwrap();
            assert!(request.starts_with(target), "{url}: {request}");
        }
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
                query: None,
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
                query: None,
                headers: &[],
                body: None,
            },
            &mut buffer,
            || {
                delays.set(delays.get() + 1);
                core::future::ready(())
            },
        ));

        assert!(matches!(
            result,
            Err(Error::TcpError(embedded_io_async::ErrorKind::Other))
        ));
        // max_retries counts total attempts: 2 failures, 1 delay in between.
        assert_eq!(delays.get(), 1);
    }
}
