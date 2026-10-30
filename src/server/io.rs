use crate::{
    codec::{self, Framing, FramingError},
    error::Error,
    handler::HttpHandler,
    header::{HttpHeader, headers::CONNECTION, mime_types},
    protocol::{self, DOUBLE_CRLF_LEN},
    request::HttpRequest,
    response::{HttpResponse, ResponseBody},
    status_code::StatusCode,
};
use embedded_io_async::{Read, Write};
use heapless::Vec;

const DEFAULT_REQUEST_SIZE: usize = 4096;
const DEFAULT_RESPONSE_SIZE: usize = 4096;
const SMALL_REQUEST_SIZE: usize = 1024;
const SMALL_RESPONSE_SIZE: usize = 1024;

/// Transport-generic HTTP server for already-accepted streams.
pub struct HttpServer<
    const REQ_SIZE: usize = DEFAULT_REQUEST_SIZE,
    const MAX_RESPONSE_SIZE: usize = DEFAULT_RESPONSE_SIZE,
>;

impl<const REQ_SIZE: usize, const MAX_RESPONSE_SIZE: usize>
    HttpServer<REQ_SIZE, MAX_RESPONSE_SIZE>
{
    /// Create a new transport-generic server.
    ///
    /// Buffer sizes come from the type parameters; see [`DefaultHttpServer`]
    /// and [`SmallHttpServer`] for ready-made configurations.
    #[must_use]
    pub const fn new() -> Self {
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

impl Default for HttpServer<DEFAULT_REQUEST_SIZE, DEFAULT_RESPONSE_SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

/// Type alias for `HttpServer` with default request and response buffer sizes.
pub type DefaultHttpServer = HttpServer<DEFAULT_REQUEST_SIZE, DEFAULT_RESPONSE_SIZE>;

/// Type alias for `HttpServer` with smaller request and response buffer sizes.
pub type SmallHttpServer = HttpServer<SMALL_REQUEST_SIZE, SMALL_RESPONSE_SIZE>;

/// Handle a single HTTP server connection over a generic async stream.
///
/// # Errors
///
/// Returns an error if stream IO fails, request parsing fails, handler execution
/// fails, or response serialization exceeds the configured response buffer.
///
/// Where the peer can still be answered, it first gets an error response:
/// `400` for a malformed request, `413` for a body that does not fit the
/// request buffer, `431` for headers that do not fit, and `500` when the
/// handler fails or its response cannot be serialized. The original error is
/// still returned. Nothing is sent when the stream itself failed or the peer
/// closed the connection mid-request.
pub async fn handle_http_connection<S, H>(stream: &mut S, handler: &mut H) -> Result<(), Error>
where
    S: Read + Write,
    H: HttpHandler,
{
    handle_http_connection_with_sizes::<S, H, DEFAULT_REQUEST_SIZE, DEFAULT_RESPONSE_SIZE>(
        stream, handler,
    )
    .await
}

/// Handle a single HTTP server connection with custom request/response buffer sizes.
///
/// # Errors
///
/// Returns an error if stream IO fails, request parsing fails, handler execution
/// fails, or response serialization exceeds the configured response buffer.
///
/// Where the peer can still be answered, it first gets an error response:
/// `400` for a malformed request, `413` for a body that does not fit the
/// request buffer, `431` for headers that do not fit, and `500` when the
/// handler fails or its response cannot be serialized. The original error is
/// still returned. Nothing is sent when the stream itself failed or the peer
/// closed the connection mid-request.
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
    let total_read = match read_request(stream, &mut request_buffer).await {
        Ok(total_read) => total_read,
        Err(failure) => {
            if let Some(status) = failure.reply {
                send_status::<S, MAX_RESPONSE_SIZE>(stream, status).await;
            }
            return Err(failure.error);
        }
    };

    // A chunked request body is decoded in place, so handlers see the payload.
    let total_read = match codec::dechunk(&mut request_buffer, total_read) {
        Ok(total_read) => total_read,
        Err(error) => {
            send_status::<S, MAX_RESPONSE_SIZE>(stream, StatusCode::BadRequest).await;
            return Err(error);
        }
    };

    let request = match HttpRequest::try_from(&request_buffer[..total_read]) {
        Ok(request) => request,
        Err(error) => {
            send_status::<S, MAX_RESPONSE_SIZE>(stream, StatusCode::BadRequest).await;
            return Err(error);
        }
    };

    let response = handler.handle_request(&request).await.map_or_else(
        |_| {
            text_error::<MAX_RESPONSE_SIZE>(
                StatusCode::InternalServerError,
                "Internal Server Error",
            )
        },
        |response| response.build_bytes::<MAX_RESPONSE_SIZE>(),
    );
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            // The response could not be serialized (for example an invalid
            // header or a body larger than the response buffer).
            send_status::<S, MAX_RESPONSE_SIZE>(stream, StatusCode::InternalServerError).await;
            return Err(error);
        }
    };

    stream.write_all(&response).await.map_err(Error::io)?;
    stream.flush().await.map_err(Error::io)
}

/// Why a request could not be read, and the status to answer with, if any.
struct ReadFailure {
    error: Error,
    reply: Option<StatusCode>,
}

impl ReadFailure {
    /// The peer cannot (or need not) be answered.
    const fn silent(error: Error) -> Self {
        Self { error, reply: None }
    }

    const fn reply(error: Error, status: StatusCode) -> Self {
        Self {
            error,
            reply: Some(status),
        }
    }
}

/// Read one complete HTTP request (headers plus any `Content-Length` body).
///
/// A body is delimited by `Content-Length` or by chunked transfer coding.
/// Requests with neither are considered complete once the headers end; the
/// peer is expected to close the connection after such a request. Chunked
/// bodies are returned as received; the caller decodes them with
/// [`codec::dechunk`].
///
/// A declared `Content-Length` that cannot fit in `buf` is rejected as soon as
/// the headers arrive, instead of reading until the buffer fills.
///
/// # Errors
///
/// - [`Error::TcpError`] if the stream fails.
/// - [`Error::InvalidResponse`] if the peer closes the connection before the
///   request headers (or a declared body) are complete.
/// - [`Error::BufferOverflow`] if the request does not fit in `buf`, answered
///   with `413` when the headers were read and `431` when they were not.
async fn read_request<S>(stream: &mut S, buf: &mut [u8]) -> Result<usize, ReadFailure>
where
    S: Read,
{
    let mut total_read = 0;
    let mut header_end = None;
    let mut framing = None;

    while total_read < buf.len() {
        let n = stream
            .read(&mut buf[total_read..])
            .await
            .map_err(|error| ReadFailure::silent(Error::io(error)))?;
        if n == 0 {
            return Err(ReadFailure::silent(Error::InvalidResponse(
                "Incomplete request",
            )));
        }
        total_read += n;

        if header_end.is_none() {
            header_end = protocol::find_double_crlf(&buf[..total_read]);
        }

        if let Some(hdr_end) = header_end {
            let framing = if let Some(framing) = framing {
                framing
            } else {
                let found = codec::request_framing(&buf[..hdr_end]).map_err(framing_failure)?;
                framing = Some(found);
                found
            };
            let body_start = hdr_end + DOUBLE_CRLF_LEN;
            match framing {
                // Headers are complete and no body is expected.
                Framing::None => return Ok(total_read),
                Framing::Length(content_length) => {
                    let needed = body_start.saturating_add(content_length);
                    if needed > buf.len() {
                        return Err(ReadFailure::reply(
                            Error::BufferOverflow,
                            StatusCode::RequestEntityTooLarge,
                        ));
                    }
                    if total_read >= needed {
                        return Ok(total_read);
                    }
                }
                Framing::Chunked => {
                    if codec::chunked_body_complete(&buf[body_start..total_read]) {
                        return Ok(total_read);
                    }
                }
            }
        }
    }

    // The buffer filled up: with the headers if they never ended, otherwise
    // with a chunked body of unknown length.
    let status = if header_end.is_some() {
        StatusCode::RequestEntityTooLarge
    } else {
        StatusCode::RequestHeaderFieldsTooLarge
    };
    Err(ReadFailure::reply(Error::BufferOverflow, status))
}

/// Map a request framing error to the response the peer should get.
const fn framing_failure(error: FramingError) -> ReadFailure {
    match error {
        FramingError::Invalid(reason) => {
            ReadFailure::reply(Error::InvalidResponse(reason), StatusCode::BadRequest)
        }
        FramingError::UnsupportedCoding => ReadFailure::reply(
            Error::InvalidResponse("Unsupported transfer coding"),
            StatusCode::NotImplemented,
        ),
    }
}

/// Best-effort plain-text error response; a failure to send it is ignored
/// because the caller already has an error to report.
async fn send_status<S, const MAX_RESPONSE_SIZE: usize>(stream: &mut S, status: StatusCode)
where
    S: Write,
{
    if let Ok(response) = text_error::<MAX_RESPONSE_SIZE>(status, status.text())
        && stream.write_all(&response).await.is_ok()
    {
        let _ = stream.flush().await;
    }
}

fn text_error<const MAX_RESPONSE_SIZE: usize>(
    status: StatusCode,
    body: &str,
) -> Result<Vec<u8, MAX_RESPONSE_SIZE>, Error> {
    let mut headers = Vec::new();
    let _ = headers.push(HttpHeader::content_type(mime_types::TEXT));
    let _ = headers.push(HttpHeader::new(CONNECTION, "close"));
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
    use crate::SimpleHandler;
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
        let server = HttpServer::<128, 512>::new();

        futures_lite::future::block_on(server.handle_connection(&mut stream, &mut handler))
            .unwrap();

        let response = core::str::from_utf8(&stream.output).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.contains("Hello from nanofish HTTP server"));
    }

    /// Handler that asserts the parsed request body equals `hello`.
    struct BodyCheckHandler;

    impl HttpHandler for BodyCheckHandler {
        async fn handle_request(
            &mut self,
            request: &HttpRequest<'_>,
        ) -> Result<HttpResponse<'_>, Error> {
            let mut headers = Vec::new();
            let _ = headers.push(HttpHeader::content_type(mime_types::TEXT));
            let body = if request.body == b"hello".as_slice() && request.content_length() == Some(5)
            {
                "body-ok"
            } else {
                "body-bad"
            };
            Ok(HttpResponse {
                status_code: StatusCode::Ok,
                headers,
                body: ResponseBody::Text(body),
            })
        }
    }

    #[test]
    fn test_handle_post_with_content_length_body() {
        let request = b"POST /echo HTTP/1.1\r\nHost: example.com\r\nContent-Length: 5\r\n\r\nhello";
        let mut stream = MockStream::<128, 512>::new(request);
        let mut handler = BodyCheckHandler;

        futures_lite::future::block_on(handle_http_connection_with_sizes::<_, _, 128, 512>(
            &mut stream,
            &mut handler,
        ))
        .unwrap();

        let response = core::str::from_utf8(&stream.output).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.ends_with("body-ok"));
    }

    /// Handler that reflects request input into a response header.
    struct ReflectHandler;

    impl HttpHandler for ReflectHandler {
        async fn handle_request(
            &mut self,
            _request: &HttpRequest<'_>,
        ) -> Result<HttpResponse<'_>, Error> {
            let mut headers = Vec::new();
            let _ = headers.push(HttpHeader::new("Location", "/a\r\nSet-Cookie: evil=1"));
            Ok(HttpResponse {
                status_code: StatusCode::Found,
                headers,
                body: ResponseBody::Empty,
            })
        }
    }

    #[test]
    fn test_response_header_injection_gets_500() {
        let request = b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\n";
        let mut stream = MockStream::<128, 512>::new(request);
        let mut handler = ReflectHandler;

        let err = futures_lite::future::block_on(
            handle_http_connection_with_sizes::<_, _, 128, 512>(&mut stream, &mut handler),
        )
        .unwrap_err();

        assert!(matches!(err, Error::HeaderError("Invalid header value")));
        // The injected header never reaches the peer; it gets a 500 instead.
        let response = core::str::from_utf8(&stream.output).unwrap();
        assert!(response.starts_with("HTTP/1.1 500 Internal Server Error\r\n"));
        assert!(!response.contains("Set-Cookie"));
    }

    #[test]
    fn test_incomplete_request_body_is_rejected() {
        // Declares Content-Length: 100 but the peer closes after 5 body bytes.
        let request =
            b"POST /echo HTTP/1.1\r\nHost: example.com\r\nContent-Length: 100\r\n\r\nhello";
        let mut stream = MockStream::<128, 512>::new(request);
        let mut handler = BodyCheckHandler;

        let err = futures_lite::future::block_on(
            handle_http_connection_with_sizes::<_, _, 256, 512>(&mut stream, &mut handler),
        )
        .unwrap_err();

        assert!(matches!(err, Error::InvalidResponse("Incomplete request")));
        assert!(stream.output.is_empty());
    }

    #[test]
    fn test_incomplete_request_headers_are_rejected() {
        let request = b"GET / HTTP/1.1\r\nHost: exa";
        let mut stream = MockStream::<128, 512>::new(request);
        let mut handler = SimpleHandler;

        let err = futures_lite::future::block_on(
            handle_http_connection_with_sizes::<_, _, 128, 512>(&mut stream, &mut handler),
        )
        .unwrap_err();

        assert!(matches!(err, Error::InvalidResponse("Incomplete request")));
    }

    #[test]
    fn test_request_larger_than_buffer_is_rejected() {
        let mut raw = Vec::<u8, 256>::new();
        raw.extend_from_slice(b"GET /").unwrap();
        for _ in 0..200 {
            raw.push(b'a').unwrap();
        }
        raw.extend_from_slice(b" HTTP/1.1\r\nHost: h\r\n\r\n")
            .unwrap();

        let mut stream = MockStream::<256, 512>::new(&raw);
        let mut handler = SimpleHandler;

        let err = futures_lite::future::block_on(
            handle_http_connection_with_sizes::<_, _, 128, 512>(&mut stream, &mut handler),
        )
        .unwrap_err();

        assert!(matches!(err, Error::BufferOverflow));
        assert_status(&stream.output, "431 Request Header Fields Too Large");
    }

    #[test]
    fn test_absurd_content_length_does_not_overflow() {
        // usize::MAX must not overflow the completion check in debug builds.
        let request =
            b"POST /echo HTTP/1.1\r\nHost: h\r\nContent-Length: 18446744073709551615\r\n\r\nhi";
        let mut stream = MockStream::<128, 512>::new(request);
        let mut handler = BodyCheckHandler;

        let err = futures_lite::future::block_on(
            handle_http_connection_with_sizes::<_, _, 128, 512>(&mut stream, &mut handler),
        )
        .unwrap_err();

        assert!(matches!(err, Error::BufferOverflow));
        assert_status(&stream.output, "413 Request Entity Too Large");
    }

    /// Assert that `output` is an error response with the given status text
    /// that tells the peer the connection is closing.
    fn assert_status(output: &[u8], status: &str) {
        let response = core::str::from_utf8(output).unwrap();
        assert!(
            response.starts_with(
                heapless::format!(64; "HTTP/1.1 {status}\r\n")
                    .unwrap()
                    .as_str()
            ),
            "{response}"
        );
        assert!(response.contains("Connection: close\r\n"), "{response}");
    }

    #[test]
    fn test_content_length_larger_than_buffer_is_rejected_early() {
        // Only the headers have arrived, but the declared body can never fit
        // in the 128-byte request buffer.
        let request = b"POST /echo HTTP/1.1\r\nHost: h\r\nContent-Length: 1000\r\n\r\n";
        let mut stream = MockStream::<128, 512>::new(request);
        let mut handler = BodyCheckHandler;

        let err = futures_lite::future::block_on(
            handle_http_connection_with_sizes::<_, _, 128, 512>(&mut stream, &mut handler),
        )
        .unwrap_err();

        assert!(matches!(err, Error::BufferOverflow));
        assert_status(&stream.output, "413 Request Entity Too Large");
    }

    /// Reads at most `max` bytes at a time, like a slow peer.
    struct SlowStream<const IN: usize, const OUT: usize> {
        inner: MockStream<IN, OUT>,
        max: usize,
    }

    impl<const IN: usize, const OUT: usize> ErrorType for SlowStream<IN, OUT> {
        type Error = Infallible;
    }

    impl<const IN: usize, const OUT: usize> Read for SlowStream<IN, OUT> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            let n = buf.len().min(self.max);
            self.inner.read(&mut buf[..n]).await
        }
    }

    impl<const IN: usize, const OUT: usize> Write for SlowStream<IN, OUT> {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            self.inner.write(buf).await
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            self.inner.flush().await
        }
    }

    /// Handler that reports whether the request body equals `hello`.
    struct HelloBodyHandler;

    impl HttpHandler for HelloBodyHandler {
        async fn handle_request(
            &mut self,
            request: &HttpRequest<'_>,
        ) -> Result<HttpResponse<'_>, Error> {
            let body = if request.body == b"hello".as_slice() {
                "body-ok"
            } else {
                "body-bad"
            };
            Ok(HttpResponse {
                status_code: StatusCode::Ok,
                headers: Vec::new(),
                body: ResponseBody::Text(body),
            })
        }
    }

    const CHUNKED_POST: &[u8] = b"POST /echo HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nhel\r\n2\r\nlo\r\n0\r\n\r\n";

    #[test]
    fn test_chunked_request_body_is_decoded() {
        let mut stream = MockStream::<256, 512>::new(CHUNKED_POST);
        let mut handler = HelloBodyHandler;

        futures_lite::future::block_on(handle_http_connection_with_sizes::<_, _, 256, 512>(
            &mut stream,
            &mut handler,
        ))
        .unwrap();

        let response = core::str::from_utf8(&stream.output).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.ends_with("body-ok"), "{response}");
    }

    #[test]
    fn test_chunked_request_body_arriving_in_small_pieces() {
        // Every read ends mid-frame at some point; the server must keep reading
        // until the last chunk and its terminating line are in.
        for max in [1, 2, 3, 5, 7] {
            let mut stream = SlowStream {
                inner: MockStream::<256, 512>::new(CHUNKED_POST),
                max,
            };
            let mut handler = HelloBodyHandler;

            futures_lite::future::block_on(handle_http_connection_with_sizes::<_, _, 256, 512>(
                &mut stream,
                &mut handler,
            ))
            .unwrap();

            let response = core::str::from_utf8(&stream.inner.output).unwrap();
            assert!(response.ends_with("body-ok"), "max={max}: {response}");
        }
    }

    #[test]
    fn test_chunked_request_with_trailers() {
        let request = b"POST /echo HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\nX-Sum: 1\r\n\r\n";
        let mut stream = MockStream::<256, 512>::new(request);
        let mut handler = HelloBodyHandler;

        futures_lite::future::block_on(handle_http_connection_with_sizes::<_, _, 256, 512>(
            &mut stream,
            &mut handler,
        ))
        .unwrap();

        let response = core::str::from_utf8(&stream.output).unwrap();
        assert!(response.ends_with("body-ok"), "{response}");
    }

    #[test]
    fn test_chunked_request_larger_than_buffer_gets_413() {
        // 100 bytes of chunk data cannot fit a 128-byte buffer with the headers.
        let mut raw = Vec::<u8, 256>::new();
        raw.extend_from_slice(
            b"POST /echo HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n64\r\n",
        )
        .unwrap();
        for _ in 0..100 {
            raw.push(b'a').unwrap();
        }
        raw.extend_from_slice(b"\r\n0\r\n\r\n").unwrap();
        let mut stream = MockStream::<256, 512>::new(&raw);
        let mut handler = HelloBodyHandler;

        let err = futures_lite::future::block_on(
            handle_http_connection_with_sizes::<_, _, 128, 512>(&mut stream, &mut handler),
        )
        .unwrap_err();

        assert!(matches!(err, Error::BufferOverflow));
        assert_status(&stream.output, "413 Request Entity Too Large");
    }

    #[test]
    fn test_malformed_chunked_request_gets_400() {
        let request = b"POST /echo HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\nhello\r\n0\r\n\r\n";
        let mut stream = MockStream::<256, 512>::new(request);
        let mut handler = HelloBodyHandler;

        let err = futures_lite::future::block_on(
            handle_http_connection_with_sizes::<_, _, 256, 512>(&mut stream, &mut handler),
        )
        .unwrap_err();

        assert!(matches!(err, Error::InvalidResponse("Invalid chunk size")));
        assert_status(&stream.output, "400 Bad Request");
    }

    #[test]
    fn test_ambiguous_request_framing_gets_400() {
        for request in [
            &b"POST /echo HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\nContent-Length: 5\r\n\r\n5\r\nhello\r\n0\r\n\r\n"[..],
            b"POST /echo HTTP/1.1\r\nHost: h\r\nContent-Length: 5\r\nContent-Length: 6\r\n\r\nhello!",
            b"POST /echo HTTP/1.1\r\nHost: h\r\nContent-Length: abc\r\n\r\nhello",
            b"POST /echo HTTP/1.1\r\nHost: h\r\nContent-Length: +5\r\n\r\nhello",
        ] {
            let mut stream = MockStream::<256, 512>::new(request);
            let mut handler = HelloBodyHandler;

            let err = futures_lite::future::block_on(
                handle_http_connection_with_sizes::<_, _, 256, 512>(&mut stream, &mut handler),
            )
            .unwrap_err();

            assert!(matches!(err, Error::InvalidResponse(_)));
            assert_status(&stream.output, "400 Bad Request");
        }
    }

    #[test]
    fn test_unsupported_transfer_coding_gets_501() {
        for coding in ["gzip", "gzip, chunked", "deflate"] {
            let mut request = Vec::<u8, 256>::new();
            request
                .extend_from_slice(b"POST /echo HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: ")
                .unwrap();
            request.extend_from_slice(coding.as_bytes()).unwrap();
            request
                .extend_from_slice(b"\r\n\r\n5\r\nhello\r\n0\r\n\r\n")
                .unwrap();
            let mut stream = MockStream::<256, 512>::new(&request);
            let mut handler = HelloBodyHandler;

            let err =
                futures_lite::future::block_on(
                    handle_http_connection_with_sizes::<_, _, 256, 512>(&mut stream, &mut handler),
                )
                .unwrap_err();

            assert!(
                matches!(err, Error::InvalidResponse("Unsupported transfer coding")),
                "{coding}"
            );
            assert_status(&stream.output, "501 Not Implemented");
        }
    }

    #[test]
    fn test_repeated_identical_content_length_is_accepted() {
        let request = b"POST /echo HTTP/1.1\r\nHost: h\r\nContent-Length: 5\r\nContent-Length: 5\r\n\r\nhello";
        let mut stream = MockStream::<256, 512>::new(request);
        let mut handler = HelloBodyHandler;

        futures_lite::future::block_on(handle_http_connection_with_sizes::<_, _, 256, 512>(
            &mut stream,
            &mut handler,
        ))
        .unwrap();

        let response = core::str::from_utf8(&stream.output).unwrap();
        assert!(response.ends_with("body-ok"), "{response}");
    }

    #[test]
    fn test_malformed_request_gets_400() {
        for request in [
            &b"GARBAGE\r\n\r\n"[..],
            b"FETCH / HTTP/1.1\r\nHost: h\r\n\r\n",
            b"GET\r\n\r\n",
        ] {
            let mut stream = MockStream::<128, 512>::new(request);
            let mut handler = SimpleHandler;

            let err =
                futures_lite::future::block_on(
                    handle_http_connection_with_sizes::<_, _, 128, 512>(&mut stream, &mut handler),
                )
                .unwrap_err();

            assert!(matches!(err, Error::InvalidResponse(_)));
            assert_status(&stream.output, "400 Bad Request");
        }
    }

    /// Handler that always fails.
    struct FailingHandler;

    impl HttpHandler for FailingHandler {
        async fn handle_request(
            &mut self,
            _request: &HttpRequest<'_>,
        ) -> Result<HttpResponse<'_>, Error> {
            Err(Error::NoResponse)
        }
    }

    #[test]
    fn test_handler_error_gets_500() {
        let request = b"GET / HTTP/1.1\r\nHost: h\r\n\r\n";
        let mut stream = MockStream::<128, 512>::new(request);
        let mut handler = FailingHandler;

        futures_lite::future::block_on(handle_http_connection_with_sizes::<_, _, 128, 512>(
            &mut stream,
            &mut handler,
        ))
        .unwrap();

        assert_status(&stream.output, "500 Internal Server Error");
    }

    #[test]
    fn test_oversized_response_and_error_response_return_error() {
        // The `GET /` page does not fit a 64-byte response buffer and neither
        // does the 500, so nothing is sent but the error is still returned.
        let request = b"GET / HTTP/1.1\r\nHost: h\r\n\r\n";
        let mut stream = MockStream::<128, 512>::new(request);
        let mut handler = SimpleHandler;

        let err = futures_lite::future::block_on(
            handle_http_connection_with_sizes::<_, _, 128, 64>(&mut stream, &mut handler),
        )
        .unwrap_err();

        assert!(matches!(err, Error::BufferOverflow));
        assert!(stream.output.is_empty());
    }

    #[test]
    fn test_oversized_response_gets_500() {
        // The page is too large for a 160-byte response buffer, but the short
        // 500 response fits.
        let request = b"GET / HTTP/1.1\r\nHost: h\r\n\r\n";
        let mut stream = MockStream::<128, 512>::new(request);
        let mut handler = LargeBodyHandler;

        let err = futures_lite::future::block_on(
            handle_http_connection_with_sizes::<_, _, 128, 160>(&mut stream, &mut handler),
        )
        .unwrap_err();

        assert!(matches!(err, Error::BufferOverflow));
        assert_status(&stream.output, "500 Internal Server Error");
    }

    /// Handler whose response is larger than a small response buffer.
    struct LargeBodyHandler;

    impl HttpHandler for LargeBodyHandler {
        async fn handle_request(
            &mut self,
            _request: &HttpRequest<'_>,
        ) -> Result<HttpResponse<'_>, Error> {
            Ok(HttpResponse {
                status_code: StatusCode::Ok,
                headers: Vec::new(),
                body: ResponseBody::Text(
                    "0123456789012345678901234567890123456789012345678901234567890123456789\
                     0123456789012345678901234567890123456789012345678901234567890123456789",
                ),
            })
        }
    }

    #[test]
    fn test_stream_failure_gets_no_response() {
        // A broken stream cannot be answered: nothing is written.
        let mut stream = MockStream::<128, 512>::new(b"");
        let mut handler = SimpleHandler;

        let err = futures_lite::future::block_on(
            handle_http_connection_with_sizes::<_, _, 128, 512>(&mut stream, &mut handler),
        )
        .unwrap_err();

        assert!(matches!(err, Error::InvalidResponse("Incomplete request")));
        assert!(stream.output.is_empty());
    }
}
