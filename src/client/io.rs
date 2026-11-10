use core::fmt::Write as _;
use core::future::Future;
use embedded_io_async::{Read, Write};
use heapless::String;
use nanofish::{
    client::{
        BodySource,
        builder::RequestBuilder,
        stream::{BodySink, StreamedResponse},
    },
    codec::{self, BodyFraming, ChunkDecoder},
    error::Error,
    header::HttpHeader,
    method::HttpMethod,
    options::HttpClientOptions,
    protocol::{self, DEFAULT_HTTP_PORT, DEFAULT_HTTPS_PORT, DOUBLE_CRLF_LEN},
    response::HttpResponse,
};

use nanofish::client::{DEFAULT_REQUEST_SIZE, SMALL_REQUEST_SIZE};

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
        .get_header(nanofish::header::headers::CONNECTION)
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
        read_exchange(
            stream,
            request.method,
            response_buffer,
            self.options.max_retries,
            keep_alive,
            retry_delay,
        )
        .await
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

    /// Write a request head announcing chunked transfer coding, then the
    /// body from `source` (one chunk per non-empty piece), then the final
    /// marker, and flush.
    ///
    /// Once the head is written the source is being consumed: a failure
    /// mid-upload is final, and the caller drops the stream.
    async fn send_request_chunked<S, U>(
        &self,
        stream: &mut S,
        request: &HttpClientRequest<'_>,
        source: &mut U,
        keep_alive: bool,
    ) -> Result<(), Error>
    where
        S: Write,
        U: BodySource,
    {
        if request.body.is_some() {
            return Err(Error::HeaderError(
                "Buffered body and upload source are both set",
            ));
        }
        let head = codec::build_req_chunked_with::<RQ>(
            request.method,
            request.host,
            request.path,
            request.query,
            request.headers,
            keep_alive,
        )?;
        stream.write_all(head.as_bytes()).await.map_err(Error::io)?;
        loop {
            let Some(data) = source.next_chunk().await? else {
                break;
            };
            if data.is_empty() {
                continue;
            }
            let mut size_line = [0u8; 18];
            stream
                .write_all(codec::chunk_size_line(data.len(), &mut size_line))
                .await
                .map_err(Error::io)?;
            stream.write_all(data).await.map_err(Error::io)?;
            stream.write_all(protocol::CRLF).await.map_err(Error::io)?;
        }
        stream
            .write_all(protocol::CHUNKED_END_MARKER)
            .await
            .map_err(Error::io)?;
        stream.flush().await.map_err(Error::io)
    }

    /// Send a request whose body is produced by `source`, with chunked
    /// transfer coding, over an already-connected stream.
    ///
    /// The request needs no `Content-Length`: each piece the source yields
    /// goes out as one chunk, so a body of any size is sent through bounded
    /// memory. An empty source sends the final marker only, which is a legal
    /// body, distinct from `Content-Length: 0`. Any method may carry a
    /// chunked body. `request.body` must be `None`; the body comes from the
    /// source.
    ///
    /// No `Expect: 100-continue` is sent; the body goes out unconditionally.
    /// Servers that answer `411 Length Required` refuse unknown lengths
    /// outright; chunked transfer coding is the answer to that, not
    /// buffering.
    ///
    /// # Errors
    ///
    /// - [`Error::HeaderError`] if `request.body` is set, or `request.headers`
    ///   contain `Content-Length` or `Transfer-Encoding`.
    /// - Any error the source returns, or [`HttpClient::request`] otherwise.
    pub async fn request_upload<'b, S, U>(
        &self,
        stream: &mut S,
        request: HttpClientRequest<'_>,
        source: &mut U,
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error>
    where
        S: Read + Write,
        U: BodySource,
    {
        self.request_upload_with_retry_delay(stream, request, source, response_buffer, || {
            core::future::ready(())
        })
        .await
    }

    /// Like [`HttpClient::request_upload`], sleeping via `retry_delay`
    /// between read retries.
    ///
    /// The retries are read retries, as in
    /// [`HttpClient::request_with_retry_delay`]: once the head is written the
    /// source is consumed, so a failure mid-upload is final and the connection
    /// must be dropped.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpClient::request_upload`].
    pub async fn request_upload_with_retry_delay<'b, S, U, D, Fut>(
        &self,
        stream: &mut S,
        request: HttpClientRequest<'_>,
        source: &mut U,
        response_buffer: &'b mut [u8],
        retry_delay: D,
    ) -> Result<(HttpResponse<'b>, usize), Error>
    where
        S: Read + Write,
        U: BodySource,
        D: Fn() -> Fut,
        Fut: Future<Output = ()>,
    {
        let exchange = self
            .exchange_upload_with_retry_delay(
                stream,
                request,
                source,
                response_buffer,
                false,
                retry_delay,
            )
            .await?;
        Ok((exchange.response, exchange.used))
    }

    /// Send a request whose body is produced by `source` and report whether
    /// the connection can carry another.
    ///
    /// A chunked upload is fully delimited, so with `keep_alive` the
    /// connection can be reused when the response allows it; see
    /// [`HttpClient::exchange`]. Redirects are not followed: a `307`/`308`
    /// would ask for the same body again, but the source is consumed.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpClient::request_upload`].
    pub async fn exchange_upload<'b, S, U>(
        &self,
        stream: &mut S,
        request: HttpClientRequest<'_>,
        source: &mut U,
        response_buffer: &'b mut [u8],
        keep_alive: bool,
    ) -> Result<Exchange<'b>, Error>
    where
        S: Read + Write,
        U: BodySource,
    {
        self.exchange_upload_with_retry_delay(
            stream,
            request,
            source,
            response_buffer,
            keep_alive,
            || core::future::ready(()),
        )
        .await
    }

    /// Like [`HttpClient::exchange_upload`], sleeping via `retry_delay`
    /// between read retries.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpClient::request_upload`].
    pub async fn exchange_upload_with_retry_delay<'b, S, U, D, Fut>(
        &self,
        stream: &mut S,
        request: HttpClientRequest<'_>,
        source: &mut U,
        response_buffer: &'b mut [u8],
        keep_alive: bool,
        retry_delay: D,
    ) -> Result<Exchange<'b>, Error>
    where
        S: Read + Write,
        U: BodySource,
        D: Fn() -> Fut,
        Fut: Future<Output = ()>,
    {
        self.send_request_chunked(stream, &request, source, keep_alive)
            .await?;
        read_exchange(
            stream,
            request.method,
            response_buffer,
            self.options.max_retries,
            keep_alive,
            retry_delay,
        )
        .await
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

        let mut retries = attempts(self.options.max_retries);
        let (head_end, mut held) =
            read_final_head(stream, buffer, &mut retries, &mut retry_delay).await?;

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

/// Read until `buffer` holds the head of the final response, dropping any
/// interim (`1xx`) responses before it.
///
/// Returns where the head ends and how many bytes the buffer holds (the head
/// and whatever part of the body came with it).
///
/// # Errors
///
/// - [`Error::BufferOverflow`] if the head does not fit in `buffer`.
/// - [`Error::NoResponse`] if the connection closes before any byte arrives.
/// - [`Error::InvalidResponse`] if it closes in the middle of a head.
/// - The read errors of [`read_retrying`].
async fn read_final_head<S, D, Fut>(
    stream: &mut S,
    buffer: &mut [u8],
    retries: &mut usize,
    retry_delay: &mut D,
) -> Result<(usize, usize), Error>
where
    S: Read,
    D: FnMut() -> Fut,
    Fut: Future<Output = ()>,
{
    let mut held = 0;
    let mut searched: usize = 0;
    let mut received = false;
    loop {
        let from = searched.saturating_sub(DOUBLE_CRLF_LEN - 1);
        if let Some(pos) = protocol::find_double_crlf(&buffer[from..held]) {
            let head_end = from + pos + DOUBLE_CRLF_LEN;
            if !codec::is_interim(&buffer[..head_end]) {
                return Ok((head_end, held));
            }
            // `100 Continue` and the like precede the final response; drop
            // them and look for the next head.
            buffer.copy_within(head_end..held, 0);
            held -= head_end;
            searched = 0;
            continue;
        }
        searched = held;

        if held == buffer.len() {
            return Err(Error::BufferOverflow);
        }
        let n = read_retrying(stream, &mut buffer[held..], retries, retry_delay).await?;
        if n == 0 {
            return Err(if received {
                Error::InvalidResponse("Incomplete response head")
            } else {
                Error::NoResponse
            });
        }
        received = true;
        held += n;
    }
}

/// The number of read attempts for a `max_retries` option: at least one, so
/// `0` still reads the response instead of failing without trying.
const fn attempts(max_retries: usize) -> usize {
    if max_retries == 0 { 1 } else { max_retries }
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

/// Read the response to a sent request, parse it, and decide whether the
/// connection can carry another request.
///
/// Shared by the buffered and the chunked-upload exchange paths, so both see
/// responses exactly the same way: read to the end of the framing, reject a
/// malformed `Content-Length`, decode chunked coding, parse, and keep the
/// connection only when `keep_alive` was asked for, the framing (not the
/// connection close) delimited the response, the peer is HTTP/1.1 and it did
/// not ask to close.
async fn read_exchange<'b, S, D, Fut>(
    stream: &mut S,
    method: HttpMethod,
    response_buffer: &'b mut [u8],
    max_retries: usize,
    keep_alive: bool,
    retry_delay: D,
) -> Result<Exchange<'b>, Error>
where
    S: Read,
    D: Fn() -> Fut,
    Fut: Future<Output = ()>,
{
    let read = read_response(stream, method, response_buffer, max_retries, retry_delay).await?;
    let end = read.end;
    codec::check_response_framing(&response_buffer[..end])?;
    let http11 = response_buffer[..end].starts_with(b"HTTP/1.1 ");
    let end = if read.chunked {
        codec::dechunk(response_buffer, end)?
    } else {
        end
    };
    let response = codec::parse_resp(&response_buffer[..end])?;
    let reusable = keep_alive && read.framed && http11 && !wants_close(&response);
    Ok(Exchange {
        response,
        used: end,
        reusable,
    })
}

/// What [`read_response`] found.
struct ReadResponse {
    /// Where the response ends in the buffer. Bytes the peer sent beyond its
    /// declared length are not part of it.
    end: usize,
    /// Whether the end was delimited by the framing (`true`) rather than by
    /// the connection closing.
    framed: bool,
    /// Whether the body uses chunked framing.
    chunked: bool,
}

/// Read a full HTTP response into `buffer`, retrying transient read errors.
///
/// `retry_delay` is awaited between retries. A response is complete once its
/// headers plus `Content-Length` body bytes or the chunked terminator arrive;
/// a body without either is delimited by connection close. Responses to `HEAD`
/// requests and `204` and `304` responses have no body and are complete at the
/// end of the headers. Interim `1xx` responses (`100 Continue`, `103 Early
/// Hints`) are dropped from the buffer: the final response follows them.
///
/// # Errors
///
/// - [`Error::NoResponse`] if nothing was read.
/// - [`Error::TcpError`] if `max_retries` read attempts fail, with the kind of
///   the last failure.
/// - [`Error::InvalidResponse`] if the peer closes the connection before a
///   length-delimited (or chunked) body is complete, or after interim
///   responses only.
/// - [`Error::BufferOverflow`] if the buffer fills before the response is
///   complete.
async fn read_response<S, D, Fut>(
    stream: &mut S,
    method: HttpMethod,
    response_buffer: &mut [u8],
    max_retries: usize,
    retry_delay: D,
) -> Result<ReadResponse, Error>
where
    S: Read,
    D: Fn() -> Fut,
    Fut: Future<Output = ()>,
{
    let mut total_read = 0;
    let mut retries = attempts(max_retries);
    let mut received = false;
    let mut saw_eof = false;
    let mut scan = codec::ResponseScan::new(method);

    loop {
        if total_read > 0 {
            if scan.is_complete(&response_buffer[..total_read]) {
                return Ok(ReadResponse {
                    end: scan.response_end(total_read),
                    framed: true,
                    chunked: scan.is_chunked(),
                });
            }
            if let Some(interim_end) = scan.interim_end() {
                response_buffer.copy_within(interim_end..total_read, 0);
                total_read -= interim_end;
                scan = codec::ResponseScan::new(method);
                continue;
            }
        }
        if total_read == response_buffer.len() {
            break;
        }
        match stream.read(&mut response_buffer[total_read..]).await {
            Ok(0) => {
                saw_eof = true;
                break;
            }
            Ok(n) => {
                total_read += n;
                received = true;
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

    if !received {
        return Err(Error::NoResponse);
    }

    if saw_eof {
        if scan.declares_length() || total_read == 0 {
            return Err(Error::InvalidResponse("Incomplete response body"));
        }
        // No length was declared: connection close delimits the body.
        return Ok(ReadResponse {
            end: total_read,
            framed: false,
            chunked: false,
        });
    }

    // The buffer filled before the response was complete.
    Err(Error::BufferOverflow)
}

#[cfg(test)]
mod tests {
    use core::convert::Infallible;
    use embedded_io_async::ErrorType;
    use heapless::Vec;
    use nanofish::client::io::*;
    use nanofish::client::{BodySource, SliceSource};
    use nanofish::{ResponseBody, StatusCode, codec};

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
        let token = nanofish::encode::bearer_auth::<32>("abc").unwrap();
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
        for _ in 0..=nanofish::protocol::MAX_HEADERS {
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
        let mut form = nanofish::encode::QueryBuilder::<64>::new();
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

    /// Yields the given pieces in order, then ends; returns an error instead
    /// of the piece at index `fail_on`.
    struct Pieces<'p> {
        script: &'p [&'p [u8]],
        next: usize,
        fail_on: Option<usize>,
    }

    impl<'p> Pieces<'p> {
        fn new(script: &'p [&'p [u8]]) -> Self {
            Self {
                script,
                next: 0,
                fail_on: None,
            }
        }
    }

    impl BodySource for Pieces<'_> {
        async fn next_chunk(&mut self) -> Result<Option<&[u8]>, Error> {
            if self.fail_on == Some(self.next) {
                return Err(Error::BufferOverflow);
            }
            let piece = self.script.get(self.next).copied();
            self.next += 1;
            Ok(piece)
        }
    }

    /// A POST to `path` whose body comes from a source, not `body`.
    fn upload_request(path: &'static str) -> HttpClientRequest<'static> {
        HttpClientRequest {
            method: HttpMethod::POST,
            host: "example.com",
            path,
            query: None,
            headers: &[],
            body: None,
        }
    }

    #[test]
    fn test_io_client_chunked_upload_wire_format() {
        let response = b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n";
        let mut stream = MockStream::<128, 256>::new(response);
        let client = HttpClient::new();
        let mut buffer = [0; 64];
        let mut source = Pieces::new(&[b"hello", b" world"]);

        let (response, _) = futures_lite::future::block_on(client.request_upload(
            &mut stream,
            HttpClientRequest {
                method: HttpMethod::POST,
                host: "example.com",
                path: "/upload",
                query: None,
                headers: &[HttpHeader::new("Content-Type", "application/octet-stream")],
                body: None,
            },
            &mut source,
            &mut buffer,
        ))
        .unwrap();

        assert_eq!(response.status_code, StatusCode::Created);
        assert_eq!(
            core::str::from_utf8(&stream.output).unwrap(),
            "POST /upload HTTP/1.1\r\n\
             Host: example.com\r\n\
             Content-Type: application/octet-stream\r\n\
             Transfer-Encoding: chunked\r\n\
             Connection: close\r\n\
             \r\n\
             5\r\nhello\r\n\
             6\r\n world\r\n\
             0\r\n\r\n"
        );
    }

    #[test]
    fn test_io_client_chunked_upload_round_trips_through_codec() {
        // What the client writes must be one complete, well-formed chunked
        // request to the codec's server side.
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
        let mut stream = MockStream::<128, 256>::new(response);
        let client = HttpClient::new();
        let mut buffer = [0; 64];
        let mut source = SliceSource::new(b"hello world", 4);

        futures_lite::future::block_on(client.request_upload(
            &mut stream,
            upload_request("/upload"),
            &mut source,
            &mut buffer,
        ))
        .unwrap();

        let mut raw = [0u8; 256];
        raw[..stream.output.len()].copy_from_slice(&stream.output);
        let len = stream.output.len();
        assert!(codec::complete(&raw[..len]));
        let total = codec::dechunk(&mut raw, len).unwrap();
        let head_end = nanofish::protocol::find_double_crlf(&raw[..total]).unwrap()
            + nanofish::protocol::DOUBLE_CRLF_LEN;
        assert_eq!(&raw[head_end..total], b"hello world");
    }

    #[test]
    fn test_io_client_chunked_upload_empty_source_sends_only_final_marker() {
        let response = b"HTTP/1.1 204 No Content\r\n\r\n";
        let mut stream = MockStream::<64, 256>::new(response);
        let client = HttpClient::new();
        let mut buffer = [0; 64];
        let mut source = SliceSource::new(b"", 8);

        futures_lite::future::block_on(client.request_upload(
            &mut stream,
            upload_request("/trigger"),
            &mut source,
            &mut buffer,
        ))
        .unwrap();

        assert_eq!(
            core::str::from_utf8(&stream.output).unwrap(),
            "POST /trigger HTTP/1.1\r\n\
             Host: example.com\r\n\
             Transfer-Encoding: chunked\r\n\
             Connection: close\r\n\
             \r\n\
             0\r\n\r\n"
        );
    }

    #[test]
    fn test_io_client_chunked_upload_skips_empty_slices() {
        let response = b"HTTP/1.1 204 No Content\r\n\r\n";
        let mut stream = MockStream::<64, 256>::new(response);
        let client = HttpClient::new();
        let mut buffer = [0; 64];
        let mut source = Pieces::new(&[b"", b"ab", b""]);

        futures_lite::future::block_on(client.request_upload(
            &mut stream,
            upload_request("/upload"),
            &mut source,
            &mut buffer,
        ))
        .unwrap();

        assert!(
            core::str::from_utf8(&stream.output)
                .unwrap()
                .ends_with("2\r\nab\r\n0\r\n\r\n"),
            "{:?}",
            stream.output
        );
    }

    #[test]
    fn test_io_client_chunked_upload_source_error_is_returned() {
        let mut stream = MockStream::<8, 256>::new(b"");
        let client = HttpClient::new();
        let mut buffer = [0; 64];
        let mut source = Pieces {
            fail_on: Some(1),
            ..Pieces::new(&[b"ab", b"cd"])
        };

        let result = futures_lite::future::block_on(client.request_upload(
            &mut stream,
            upload_request("/upload"),
            &mut source,
            &mut buffer,
        ));

        assert!(matches!(result, Err(Error::BufferOverflow)));
        // The first chunk went out, then the upload stopped short of the
        // final marker.
        let request = core::str::from_utf8(&stream.output).unwrap();
        assert!(request.ends_with("2\r\nab\r\n"), "{request}");
    }

    #[test]
    fn test_io_client_chunked_upload_rejects_buffered_body() {
        let mut stream = MockStream::<8, 256>::new(b"");
        let client = HttpClient::new();
        let mut buffer = [0; 64];
        let mut source = SliceSource::new(b"hello", 2);

        let result = futures_lite::future::block_on(client.request_upload(
            &mut stream,
            HttpClientRequest {
                method: HttpMethod::POST,
                host: "example.com",
                path: "/upload",
                query: None,
                headers: &[],
                body: Some(b"hello"),
            },
            &mut source,
            &mut buffer,
        ));

        assert!(matches!(
            result,
            Err(Error::HeaderError(
                "Buffered body and upload source are both set"
            ))
        ));
        // Nothing was sent.
        assert!(stream.output.is_empty());
    }

    #[test]
    fn test_io_client_chunked_upload_rejects_explicit_framing_headers() {
        for name in [
            "Content-Length",
            "content-length",
            "Transfer-Encoding",
            "transfer-encoding",
        ] {
            let mut stream = MockStream::<8, 256>::new(b"");
            let client = HttpClient::new();
            let mut buffer = [0; 64];
            let mut source = SliceSource::new(b"hello", 2);

            let result = futures_lite::future::block_on(client.request_upload(
                &mut stream,
                HttpClientRequest {
                    method: HttpMethod::POST,
                    host: "example.com",
                    path: "/upload",
                    query: None,
                    headers: &[HttpHeader::new(name, "5")],
                    body: None,
                },
                &mut source,
                &mut buffer,
            ));

            assert!(matches!(result, Err(Error::HeaderError(_))), "{name}");
            assert!(stream.output.is_empty(), "{name}");
        }
    }

    #[test]
    fn test_io_client_chunked_upload_keep_alive_is_reusable() {
        // A chunked upload is fully delimited, so a keep-alive connection
        // carries another request after it.
        let replies: [&[u8]; 2] = [
            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\none",
            b"HTTP/1.1 204 No Content\r\n\r\n",
        ];
        let mut stream = ReplyStream::new(&replies);
        let client = HttpClient::new();
        let mut buffer = [0; 128];

        for (path, body) in [("/1", Some("one")), ("/2", None)] {
            let mut source = SliceSource::new(b"hello world", 5);
            let exchange = futures_lite::future::block_on(client.exchange_upload(
                &mut stream,
                upload_request(path),
                &mut source,
                &mut buffer,
                true,
            ))
            .unwrap();
            assert!(exchange.reusable, "{path}");
            assert_eq!(exchange.response.body.as_str(), body, "{path}");
        }

        for (request, path) in stream.requests.iter().zip(["/1", "/2"]) {
            assert!(request.starts_with(&std::format!("POST {path} HTTP/1.1\r\n")));
            assert!(!request.contains("Connection:"), "{request}");
            assert!(request.ends_with("0\r\n\r\n"), "{request}");
        }
    }

    #[test]
    fn test_io_client_chunked_upload_no_content_length_in_head() {
        // Chunked framing does not depend on the method, and never announces
        // a length, not even for methods that carry one.
        for method in [HttpMethod::GET, HttpMethod::POST, HttpMethod::DELETE] {
            let response = b"HTTP/1.1 204 No Content\r\n\r\n";
            let mut stream = MockStream::<64, 256>::new(response);
            let client = HttpClient::new();
            let mut buffer = [0; 64];
            let mut source = Pieces::new(&[b"x"]);

            futures_lite::future::block_on(client.request_upload(
                &mut stream,
                HttpClientRequest {
                    method,
                    host: "example.com",
                    path: "/upload",
                    query: None,
                    headers: &[],
                    body: None,
                },
                &mut source,
                &mut buffer,
            ))
            .unwrap();

            let request = core::str::from_utf8(&stream.output).unwrap();
            assert!(
                request.contains("Transfer-Encoding: chunked\r\n"),
                "{method:?}: {request}"
            );
            assert!(!request.contains("Content-Length"), "{method:?}: {request}");
        }
    }

    #[test]
    fn test_io_client_chunked_upload_response_is_parsed() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
        let mut stream = MockStream::<128, 256>::new(response);
        let client = HttpClient::new();
        let mut buffer = [0; 64];
        let mut source = SliceSource::new(b"hello", 2);

        let (response, used) = futures_lite::future::block_on(client.request_upload(
            &mut stream,
            upload_request("/upload"),
            &mut source,
            &mut buffer,
        ))
        .unwrap();

        assert_eq!(response.status_code, StatusCode::Ok);
        assert_eq!(response.body.as_str(), Some("ok"));
        assert_eq!(
            used,
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".len()
        );
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

    const INTERIM_THEN_OK: &[u8] = b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 103 Early Hints\r\nLink: </a.css>\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";

    #[test]
    fn test_interim_responses_are_skipped() {
        for max in [1, 3, 1000] {
            let mut stream = Trickle {
                inner: MockStream::<256, 256>::new(INTERIM_THEN_OK),
                max,
            };
            let client = HttpClient::new();
            let mut buffer = [0; 256];
            let exchange = futures_lite::future::block_on(client.exchange(
                &mut stream,
                get_request(HttpMethod::GET),
                &mut buffer,
                true,
            ))
            .unwrap();
            assert_eq!(exchange.response.status_code, StatusCode::Ok, "max {max}");
            assert_eq!(exchange.response.body.as_str(), Some("hello"), "max {max}");
            assert!(exchange.reusable, "max {max}");
        }
    }

    #[test]
    fn test_streaming_skips_interim_responses() {
        for max in [1, 3, 1000] {
            let mut stream = Trickle {
                inner: MockStream::<256, 256>::new(INTERIM_THEN_OK),
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
            assert_eq!(streamed.status_code, StatusCode::Ok, "max {max}");
            assert_eq!(sink.body, b"hello", "max {max}");
        }
    }

    #[test]
    fn test_only_interim_response_before_close_is_an_error() {
        let mut stream = MockStream::<128, 256>::new(b"HTTP/1.1 100 Continue\r\n\r\n");
        let client = HttpClient::new();
        let mut buffer = [0; 128];
        let result = futures_lite::future::block_on(client.request(
            &mut stream,
            get_request(HttpMethod::GET),
            &mut buffer,
        ));
        assert!(
            matches!(result, Err(Error::InvalidResponse(_))),
            "{result:?}"
        );
    }

    #[test]
    fn test_switching_protocols_is_final() {
        let mut stream =
            MockStream::<128, 256>::new(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: x\r\n\r\n");
        let client = HttpClient::new();
        let mut buffer = [0; 128];
        let (response, _) = futures_lite::future::block_on(client.request(
            &mut stream,
            get_request(HttpMethod::GET),
            &mut buffer,
        ))
        .unwrap();
        assert_eq!(response.status_code, StatusCode::SwitchingProtocols);
    }

    #[test]
    fn test_bytes_past_content_length_are_not_body() {
        let mut stream =
            MockStream::<128, 256>::new(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhelloEXTRA");
        let client = HttpClient::new();
        let mut buffer = [0; 128];
        let (response, used) = futures_lite::future::block_on(client.request(
            &mut stream,
            get_request(HttpMethod::GET),
            &mut buffer,
        ))
        .unwrap();
        assert_eq!(response.body.as_str(), Some("hello"));
        assert_eq!(
            used,
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello".len()
        );

        // A body-less response has no body, whatever follows its head.
        let mut stream = MockStream::<128, 256>::new(b"HTTP/1.1 204 No Content\r\n\r\njunk");
        let mut buffer = [0; 128];
        let (response, _) = futures_lite::future::block_on(client.request(
            &mut stream,
            get_request(HttpMethod::GET),
            &mut buffer,
        ))
        .unwrap();
        assert_eq!(response.body, ResponseBody::Empty);
    }

    #[test]
    fn test_zero_max_retries_still_reads_once() {
        let mut stream =
            MockStream::<128, 256>::new(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
        let client = DefaultHttpClient::with_options(nanofish::HttpClientOptions {
            max_retries: 0,
            ..nanofish::HttpClientOptions::default()
        });
        let mut buffer = [0; 128];
        let (response, _) = futures_lite::future::block_on(client.request(
            &mut stream,
            get_request(HttpMethod::GET),
            &mut buffer,
        ))
        .unwrap();
        assert_eq!(response.body.as_str(), Some("ok"));
    }

    #[test]
    fn test_chunked_must_be_the_last_coding() {
        // `chunked, gzip` is not chunked framing: the body runs to the close.
        let raw =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked, gzip\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
        let mut stream = MockStream::<128, 256>::new(raw);
        let client = HttpClient::new();
        let mut buffer = [0; 128];
        let exchange = futures_lite::future::block_on(client.exchange(
            &mut stream,
            get_request(HttpMethod::GET),
            &mut buffer,
            true,
        ))
        .unwrap();
        assert!(!exchange.reusable);
        assert_eq!(
            exchange.response.body.as_bytes(),
            b"5\r\nhello\r\n0\r\n\r\n"
        );
    }
}
