//! Streaming responses: send the head, then the body in pieces, without
//! building the whole response in memory.

use core::fmt::Write as _;
use embedded_io_async::{Read, Write};
use heapless::{String, Vec};
use nanofish::{
    error::Error,
    header::{HttpHeader, headers},
    method::HttpMethod,
    protocol::MAX_HEADERS,
    request::HttpRequest,
    response::{HttpResponse, ResponseBody},
    server::io::{DEFAULT_REQUEST_SIZE, DEFAULT_RESPONSE_SIZE, receive_request},
    status_code::StatusCode,
};

/// How the length of a streamed response body is announced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyLength {
    /// The body is exactly this many bytes; sent as `Content-Length`.
    Known(usize),
    /// The length is not known in advance. The body is sent with chunked
    /// transfer coding, or, for an `HTTP/1.0` client that cannot decode it,
    /// delimited by closing the connection.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyState {
    /// [`ResponseWriter::send_head`] has not been called.
    NotStarted,
    /// A body of this many remaining bytes.
    Known(usize),
    /// A chunked body.
    Chunked,
    /// A body that ends when the connection closes.
    UntilClose,
    /// [`ResponseWriter::finish`] succeeded.
    Finished,
}

/// Writes one response to a connection in pieces.
///
/// Call [`send_head`](Self::send_head) once, then [`write`](Self::write) as
/// often as needed, then [`finish`](Self::finish) (the server finishes for you
/// if the handler returns `Ok`). The writer enforces the framing: it adds
/// `Content-Length` or `Transfer-Encoding: chunked` and `Connection: close`,
/// refuses to send more bytes than a declared length, and never sends a body
/// in response to a `HEAD` request.
///
/// `HEAD_SIZE` bounds the size of the serialized status line and headers.
pub struct ResponseWriter<'s, S, const HEAD_SIZE: usize = DEFAULT_RESPONSE_SIZE> {
    stream: &'s mut S,
    method: HttpMethod,
    /// The client speaks `HTTP/1.0`, which has no chunked coding.
    http10: bool,
    state: BodyState,
}

impl<'s, S, const HEAD_SIZE: usize> ResponseWriter<'s, S, HEAD_SIZE>
where
    S: Write,
{
    /// Create a writer for the response to `request`.
    pub const fn new(stream: &'s mut S, request: &HttpRequest<'_>) -> Self {
        Self {
            stream,
            method: request.method,
            http10: request.version.eq_ignore_ascii_case("HTTP/1.0"),
            state: BodyState::NotStarted,
        }
    }

    /// Whether [`send_head`](Self::send_head) has been called.
    #[must_use]
    pub fn head_sent(&self) -> bool {
        self.state != BodyState::NotStarted
    }

    /// Whether the response is complete.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.state == BodyState::Finished
    }

    /// Send the status line and headers.
    ///
    /// `Content-Length`, `Transfer-Encoding` and `Connection` are set by the
    /// writer from `length`; passing them in `headers` is an error.
    ///
    /// # Errors
    ///
    /// - [`Error::HeaderError`] if the head was already sent, a header is
    ///   invalid or reserved, or there are too many headers.
    /// - [`Error::BufferOverflow`] if the head does not fit `HEAD_SIZE`.
    /// - [`Error::TcpError`] if the stream fails.
    pub async fn send_head(
        &mut self,
        status: StatusCode,
        headers: &[HttpHeader<'_>],
        length: BodyLength,
    ) -> Result<(), Error> {
        if self.head_sent() {
            return Err(Error::HeaderError("Response head already sent"));
        }
        if headers.iter().any(|h| is_framing_header(h.name)) {
            return Err(Error::HeaderError(
                "Content-Length, Transfer-Encoding and Connection are set by the writer",
            ));
        }

        // Declared first so it outlives the headers that borrow it.
        let mut length_text = String::<20>::new();
        let mut all = Vec::<HttpHeader<'_>, MAX_HEADERS>::new();
        for header in headers {
            all.push(header.clone())
                .map_err(|_| Error::HeaderError("Too many headers"))?;
        }

        let state = match length {
            BodyLength::Known(len) => {
                write!(length_text, "{len}").map_err(|_| Error::BufferOverflow)?;
                all.push(HttpHeader::new(headers::CONTENT_LENGTH, &length_text))
                    .map_err(|_| Error::HeaderError("Too many headers"))?;
                BodyState::Known(len)
            }
            BodyLength::Unknown if self.http10 => BodyState::UntilClose,
            BodyLength::Unknown => {
                all.push(HttpHeader::new(headers::TRANSFER_ENCODING, "chunked"))
                    .map_err(|_| Error::HeaderError("Too many headers"))?;
                BodyState::Chunked
            }
        };
        all.push(HttpHeader::connection("close"))
            .map_err(|_| Error::HeaderError("Too many headers"))?;

        let head = HttpResponse {
            status_code: status,
            headers: all,
            body: ResponseBody::Empty,
        }
        .build_head_bytes::<HEAD_SIZE>()?;

        self.stream.write_all(&head).await.map_err(Error::io)?;
        self.state = state;
        Ok(())
    }

    /// Send the next piece of the body. Empty slices are ignored.
    ///
    /// For a `HEAD` request the data is counted but not sent.
    ///
    /// # Errors
    ///
    /// - [`Error::InvalidResponse`] if the head was not sent, the response
    ///   is finished, or `data` would exceed a declared [`BodyLength::Known`].
    /// - [`Error::TcpError`] if the stream fails.
    pub async fn write(&mut self, data: &[u8]) -> Result<(), Error> {
        match self.state {
            BodyState::NotStarted => {
                return Err(Error::InvalidResponse("Response head not sent"));
            }
            BodyState::Finished => return Err(Error::InvalidResponse("Response already finished")),
            BodyState::Known(remaining) => {
                if data.len() > remaining {
                    return Err(Error::InvalidResponse(
                        "Response body longer than its Content-Length",
                    ));
                }
                self.state = BodyState::Known(remaining - data.len());
            }
            BodyState::Chunked | BodyState::UntilClose => {}
        }
        if data.is_empty() || self.method == HttpMethod::HEAD {
            return Ok(());
        }

        if self.state == BodyState::Chunked {
            let mut size_line = [0u8; 18];
            let line = chunk_size_line(data.len(), &mut size_line);
            self.stream.write_all(line).await.map_err(Error::io)?;
            self.stream.write_all(data).await.map_err(Error::io)?;
            self.stream.write_all(b"\r\n").await.map_err(Error::io)
        } else {
            self.stream.write_all(data).await.map_err(Error::io)
        }
    }

    /// Complete the response and flush the stream.
    ///
    /// Finishing twice is allowed.
    ///
    /// # Errors
    ///
    /// - [`Error::InvalidResponse`] if the head was not sent, or fewer bytes
    ///   than a declared [`BodyLength::Known`] were written (the connection
    ///   must be dropped then: the peer is still waiting for the rest).
    /// - [`Error::TcpError`] if the stream fails.
    pub async fn finish(&mut self) -> Result<(), Error> {
        match self.state {
            BodyState::NotStarted => return Err(Error::InvalidResponse("Response head not sent")),
            BodyState::Finished => return Ok(()),
            BodyState::Known(remaining) if remaining > 0 => {
                return Err(Error::InvalidResponse(
                    "Response body shorter than its Content-Length",
                ));
            }
            BodyState::Chunked if self.method != HttpMethod::HEAD => {
                self.stream
                    .write_all(b"0\r\n\r\n")
                    .await
                    .map_err(Error::io)?;
            }
            _ => {}
        }
        self.stream.flush().await.map_err(Error::io)?;
        self.state = BodyState::Finished;
        Ok(())
    }
}

/// Headers whose values the writer derives from the body framing.
fn is_framing_header(name: &str) -> bool {
    [
        headers::CONTENT_LENGTH,
        headers::TRANSFER_ENCODING,
        headers::CONNECTION,
    ]
    .iter()
    .any(|reserved| name.eq_ignore_ascii_case(reserved))
}

/// Write `len` as `HEX\r\n` into `out` and return the used part.
fn chunk_size_line(len: usize, out: &mut [u8; 18]) -> &[u8] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut digits = [0u8; 16];
    let mut count = 0;
    let mut rest = len;
    loop {
        digits[count] = HEX[rest & 0xf];
        count += 1;
        rest >>= 4;
        if rest == 0 {
            break;
        }
    }
    for (index, slot) in out.iter_mut().take(count).enumerate() {
        *slot = digits[count - 1 - index];
    }
    out[count] = b'\r';
    out[count + 1] = b'\n';
    &out[..count + 2]
}

/// Handles a request by streaming the response.
///
/// Use it instead of [`HttpHandler`](nanofish::HttpHandler) for responses that
/// are large or produced incrementally, such as files read from flash, sensor
/// logs or server-sent events.
#[expect(
    async_fn_in_trait,
    reason = "no_std embedded executors are single-threaded, so no `Send` bound is wanted"
)]
pub trait StreamingHandler {
    /// Answer `request` through `response`.
    ///
    /// Return `Ok` after the response is complete (calling
    /// [`ResponseWriter::finish`] is optional). If the handler returns an error
    /// before sending the head, the client gets a `500`; after the head has been
    /// sent the connection is closed, which the client sees as a truncated
    /// response.
    async fn handle<S: Write, const HEAD_SIZE: usize>(
        &mut self,
        request: &HttpRequest<'_>,
        response: &mut ResponseWriter<'_, S, HEAD_SIZE>,
    ) -> Result<(), Error>;
}

/// Handle a single connection with a [`StreamingHandler`], using the default
/// request and head buffer sizes.
///
/// Request parsing and its error responses (`400`, `413`, `431`) work as in
/// [`handle_http_connection`](nanofish::handle_http_connection).
///
/// # Errors
///
/// Returns an error if the request cannot be read, the handler fails, or the
/// response cannot be written.
pub async fn handle_http_stream_connection<S, H>(
    stream: &mut S,
    handler: &mut H,
) -> Result<(), Error>
where
    S: Read + Write,
    H: StreamingHandler,
{
    handle_http_stream_connection_with_sizes::<S, H, DEFAULT_REQUEST_SIZE, DEFAULT_RESPONSE_SIZE>(
        stream, handler,
    )
    .await
}

/// Handle a single connection with a [`StreamingHandler`], with custom request
/// and head buffer sizes.
///
/// # Errors
///
/// Returns an error if the request cannot be read, the handler fails, or the
/// response cannot be written.
pub async fn handle_http_stream_connection_with_sizes<
    S,
    H,
    const REQ_SIZE: usize,
    const HEAD_SIZE: usize,
>(
    stream: &mut S,
    handler: &mut H,
) -> Result<(), Error>
where
    S: Read + Write,
    H: StreamingHandler,
{
    let mut request_buffer = [0; REQ_SIZE];
    let (request, _) = receive_request::<S, HEAD_SIZE>(stream, &mut request_buffer, false).await?;

    let mut writer = ResponseWriter::<S, HEAD_SIZE>::new(stream, &request);
    let result = match handler.handle(&request, &mut writer).await {
        Ok(()) => writer.finish().await,
        Err(error) => Err(error),
    };

    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            // Only an unstarted response can still be turned into an error page.
            if !writer.head_sent() {
                nanofish::server::io::send_status::<S, HEAD_SIZE>(
                    writer.stream,
                    StatusCode::InternalServerError,
                )
                .await;
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use core::convert::Infallible;
    use embedded_io_async::ErrorType;
    use nanofish::codec;
    use nanofish::server::stream::*;

    struct MockStream<const OUT: usize> {
        input: Vec<u8, 256>,
        output: Vec<u8, OUT>,
        read_pos: usize,
    }

    impl<const OUT: usize> MockStream<OUT> {
        fn new(input: &[u8]) -> Self {
            Self {
                input: Vec::from_slice(input).unwrap(),
                output: Vec::new(),
                read_pos: 0,
            }
        }

        fn output(&self) -> &str {
            core::str::from_utf8(&self.output).unwrap()
        }
    }

    impl<const OUT: usize> ErrorType for MockStream<OUT> {
        type Error = Infallible;
    }

    impl<const OUT: usize> Read for MockStream<OUT> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            let n = buf.len().min(self.input.len() - self.read_pos);
            buf[..n].copy_from_slice(&self.input[self.read_pos..self.read_pos + n]);
            self.read_pos += n;
            Ok(n)
        }
    }

    impl<const OUT: usize> Write for MockStream<OUT> {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            self.output.extend_from_slice(buf).unwrap();
            Ok(buf.len())
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    const GET: &[u8] = b"GET /log HTTP/1.1\r\nHost: h\r\n\r\n";

    #[derive(Clone, Copy)]
    enum Scenario {
        Known,
        Chunked,
        ChunkedLarge,
        NoFinish,
        WriteBeforeHead,
        TooLong,
        TooShort,
        ErrorBeforeHead,
        ErrorAfterHead,
        ReservedHeader,
        InjectedHeader,
        TwiceHead,
        FinishTwice,
    }

    struct Scripted(Scenario);

    impl StreamingHandler for Scripted {
        #[expect(
            clippy::too_many_lines,
            reason = "one scripted step per response, kept in order for readability"
        )]
        async fn handle<S: Write, const HEAD_SIZE: usize>(
            &mut self,
            _request: &HttpRequest<'_>,
            response: &mut ResponseWriter<'_, S, HEAD_SIZE>,
        ) -> Result<(), Error> {
            let text = [HttpHeader::content_type("text/plain")];
            match self.0 {
                Scenario::Known => {
                    response
                        .send_head(StatusCode::Ok, &text, BodyLength::Known(11))
                        .await?;
                    response.write(b"hello").await?;
                    response.write(b"").await?;
                    response.write(b" world").await?;
                    response.finish().await
                }
                Scenario::Chunked => {
                    response
                        .send_head(StatusCode::Ok, &text, BodyLength::Unknown)
                        .await?;
                    response.write(b"hello").await?;
                    response.write(b"").await?;
                    response.write(b" world").await?;
                    response.finish().await
                }
                Scenario::ChunkedLarge => {
                    response
                        .send_head(StatusCode::Ok, &[], BodyLength::Unknown)
                        .await?;
                    response.write(&[b'a'; 255]).await?;
                    response.write(&[b'b'; 16]).await?;
                    response.write(&[b'c'; 1]).await
                }
                Scenario::NoFinish => {
                    response
                        .send_head(StatusCode::Ok, &text, BodyLength::Unknown)
                        .await?;
                    response.write(b"abc").await
                }
                Scenario::WriteBeforeHead => response.write(b"x").await,
                Scenario::TooLong => {
                    response
                        .send_head(StatusCode::Ok, &[], BodyLength::Known(3))
                        .await?;
                    response.write(b"ab").await?;
                    response.write(b"cd").await
                }
                Scenario::TooShort => {
                    response
                        .send_head(StatusCode::Ok, &[], BodyLength::Known(10))
                        .await?;
                    response.write(b"abc").await?;
                    response.finish().await
                }
                Scenario::ErrorBeforeHead => Err(Error::NoResponse),
                Scenario::ErrorAfterHead => {
                    response
                        .send_head(StatusCode::Ok, &[], BodyLength::Unknown)
                        .await?;
                    response.write(b"partial").await?;
                    Err(Error::NoResponse)
                }
                Scenario::ReservedHeader => {
                    for name in ["Content-Length", "transfer-encoding", "Connection"] {
                        let result = response
                            .send_head(
                                StatusCode::Ok,
                                &[HttpHeader::new(name, "x")],
                                BodyLength::Unknown,
                            )
                            .await;
                        assert!(matches!(result, Err(Error::HeaderError(_))), "{name}");
                    }
                    // Rejected heads do not count as sent.
                    assert!(!response.head_sent());
                    response
                        .send_head(StatusCode::Ok, &[], BodyLength::Known(0))
                        .await
                }
                Scenario::InjectedHeader => {
                    response
                        .send_head(
                            StatusCode::Found,
                            &[HttpHeader::new("Location", "/a\r\nSet-Cookie: x=1")],
                            BodyLength::Known(0),
                        )
                        .await
                }
                Scenario::TwiceHead => {
                    response
                        .send_head(StatusCode::Ok, &[], BodyLength::Known(0))
                        .await?;
                    response
                        .send_head(StatusCode::Ok, &[], BodyLength::Known(0))
                        .await
                }
                Scenario::FinishTwice => {
                    response
                        .send_head(StatusCode::Ok, &[], BodyLength::Known(0))
                        .await?;
                    response.finish().await?;
                    assert!(response.is_finished());
                    response.finish().await?;
                    assert!(matches!(
                        response.write(b"late").await,
                        Err(Error::InvalidResponse("Response already finished"))
                    ));
                    Ok(())
                }
            }
        }
    }

    fn serve(request: &[u8], scenario: Scenario) -> (Result<(), Error>, MockStream<1024>) {
        let mut stream = MockStream::<1024>::new(request);
        let mut handler = Scripted(scenario);
        let result = futures_lite::future::block_on(handle_http_stream_connection_with_sizes::<
            _,
            _,
            256,
            256,
        >(&mut stream, &mut handler));
        (result, stream)
    }

    #[test]
    fn test_known_length_response() {
        let (result, stream) = serve(GET, Scenario::Known);
        result.unwrap();
        assert_eq!(
            stream.output(),
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 11\r\nConnection: close\r\n\r\nhello world"
        );
    }

    #[test]
    fn test_chunked_response() {
        let (result, stream) = serve(GET, Scenario::Chunked);
        result.unwrap();
        assert_eq!(
            stream.output(),
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n"
        );
    }

    #[test]
    fn test_chunked_response_round_trips_through_dechunk() {
        let (result, stream) = serve(GET, Scenario::ChunkedLarge);
        result.unwrap();
        // Chunk sizes are lowercase hex.
        assert!(
            stream.output().contains("\r\n\r\nff\r\n"),
            "{}",
            stream.output()
        );
        assert!(stream.output().contains("\r\n10\r\n"));

        let mut raw = [0u8; 1024];
        raw[..stream.output.len()].copy_from_slice(&stream.output);
        let total = codec::dechunk(&mut raw, stream.output.len()).unwrap();
        let response = codec::parse_resp(&raw[..total]).unwrap();
        let body = response.body.as_bytes();
        assert_eq!(body.len(), 255 + 16 + 1);
        assert!(body[..255].iter().all(|&b| b == b'a'));
        assert!(body[255..271].iter().all(|&b| b == b'b'));
        assert_eq!(body[271], b'c');
    }

    #[test]
    fn test_finish_is_called_for_the_handler() {
        let (result, stream) = serve(GET, Scenario::NoFinish);
        result.unwrap();
        assert!(
            stream.output().ends_with("3\r\nabc\r\n0\r\n\r\n"),
            "{}",
            stream.output()
        );
    }

    #[test]
    fn test_http10_client_gets_close_delimited_body() {
        let (result, stream) = serve(b"GET /log HTTP/1.0\r\n\r\n", Scenario::Chunked);
        result.unwrap();
        let output = stream.output();
        assert!(!output.contains("Transfer-Encoding"), "{output}");
        assert!(output.contains("Connection: close\r\n"));
        assert!(output.ends_with("\r\n\r\nhello world"), "{output}");
    }

    #[test]
    fn test_head_request_gets_no_body() {
        let head = b"HEAD /log HTTP/1.1\r\nHost: h\r\n\r\n";

        let (result, stream) = serve(head, Scenario::Known);
        result.unwrap();
        assert!(stream.output().contains("Content-Length: 11\r\n"));
        assert!(stream.output().ends_with("\r\n\r\n"), "{}", stream.output());

        let (result, stream) = serve(head, Scenario::Chunked);
        result.unwrap();
        assert!(stream.output().contains("Transfer-Encoding: chunked\r\n"));
        assert!(
            stream.output().ends_with("Connection: close\r\n\r\n"),
            "{}",
            stream.output()
        );
    }

    #[test]
    fn test_framing_errors() {
        let (result, stream) = serve(GET, Scenario::WriteBeforeHead);
        assert!(matches!(
            result,
            Err(Error::InvalidResponse("Response head not sent"))
        ));
        // The handler failed before any head: the client gets a 500.
        assert!(
            stream
                .output()
                .starts_with("HTTP/1.1 500 Internal Server Error\r\n")
        );

        let (result, stream) = serve(GET, Scenario::TooLong);
        assert!(matches!(
            result,
            Err(Error::InvalidResponse(
                "Response body longer than its Content-Length"
            ))
        ));
        // The overlong write was refused outright; only "ab" went out.
        assert!(
            stream.output().ends_with("\r\n\r\nab"),
            "{}",
            stream.output()
        );

        let (result, stream) = serve(GET, Scenario::TooShort);
        assert!(matches!(
            result,
            Err(Error::InvalidResponse(
                "Response body shorter than its Content-Length"
            ))
        ));
        assert!(stream.output().ends_with("\r\n\r\nabc"));
    }

    #[test]
    fn test_handler_error_before_and_after_head() {
        let (result, stream) = serve(GET, Scenario::ErrorBeforeHead);
        assert!(matches!(result, Err(Error::NoResponse)));
        assert!(
            stream
                .output()
                .starts_with("HTTP/1.1 500 Internal Server Error\r\n")
        );

        let (result, stream) = serve(GET, Scenario::ErrorAfterHead);
        assert!(matches!(result, Err(Error::NoResponse)));
        // No error page can follow a started response, and no terminating
        // chunk: the client sees a truncated body.
        assert!(!stream.output().contains("500"));
        assert!(
            stream.output().ends_with("7\r\npartial\r\n"),
            "{}",
            stream.output()
        );
    }

    #[test]
    fn test_reserved_and_invalid_headers() {
        let (result, stream) = serve(GET, Scenario::ReservedHeader);
        result.unwrap();
        assert!(
            stream
                .output()
                .starts_with("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n")
        );

        let (result, stream) = serve(GET, Scenario::InjectedHeader);
        assert!(matches!(
            result,
            Err(Error::HeaderError("Invalid header value"))
        ));
        assert!(!stream.output().contains("Set-Cookie"));
        assert!(
            stream
                .output()
                .starts_with("HTTP/1.1 500 Internal Server Error\r\n")
        );
    }

    #[test]
    fn test_head_cannot_be_sent_twice_and_finish_is_idempotent() {
        let (result, _) = serve(GET, Scenario::TwiceHead);
        assert!(matches!(
            result,
            Err(Error::HeaderError("Response head already sent"))
        ));

        let (result, _) = serve(GET, Scenario::FinishTwice);
        result.unwrap();
    }

    #[test]
    fn test_bad_requests_are_answered_before_the_handler() {
        let (result, stream) = serve(b"GARBAGE\r\n\r\n", Scenario::Known);
        assert!(matches!(result, Err(Error::InvalidResponse(_))));
        assert!(stream.output().starts_with("HTTP/1.1 400 Bad Request\r\n"));
    }

    #[test]
    fn test_head_too_large_for_head_buffer() {
        let mut stream = MockStream::<1024>::new(GET);
        let mut handler = Scripted(Scenario::Known);
        let result = futures_lite::future::block_on(handle_http_stream_connection_with_sizes::<
            _,
            _,
            256,
            32,
        >(&mut stream, &mut handler));
        assert!(matches!(result, Err(Error::BufferOverflow)));
    }

    #[test]
    fn test_chunk_size_line() {
        let mut out = [0u8; 18];
        for (len, expected) in [
            (1, "1\r\n"),
            (9, "9\r\n"),
            (10, "a\r\n"),
            (15, "f\r\n"),
            (16, "10\r\n"),
            (255, "ff\r\n"),
            (4096, "1000\r\n"),
            (usize::MAX, "ffffffffffffffff\r\n"),
        ] {
            assert_eq!(chunk_size_line(len, &mut out), expected.as_bytes(), "{len}");
        }
    }
}
