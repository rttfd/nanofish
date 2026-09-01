use crate::{
    codec,
    error::Error,
    handler::HttpHandler,
    header::{HttpHeader, mime_types},
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

    let request = HttpRequest::try_from(&request_buffer[..total_read])?;
    let response = handler.handle_request(&request).await.map_or_else(
        |_| {
            text_error::<MAX_RESPONSE_SIZE>(
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

/// Read one complete HTTP request (headers plus any `Content-Length` body).
///
/// Requests without `Content-Length` are considered complete once the headers
/// end; the peer is expected to close the connection after such a request.
///
/// # Errors
///
/// - [`Error::TcpError`] if the stream fails.
/// - [`Error::InvalidResponse`] if the peer closes the connection before the
///   request headers (or a declared body) are complete.
/// - [`Error::BufferOverflow`] if `buf` fills before the request is complete.
async fn read_request<S>(stream: &mut S, buf: &mut [u8]) -> Result<usize, Error>
where
    S: Read,
{
    let mut total_read = 0;
    let mut header_end = None;
    let mut saw_eof = false;
    let mut request_complete = false;

    while total_read < buf.len() {
        let n = stream
            .read(&mut buf[total_read..])
            .await
            .map_err(|_| Error::TcpError)?;
        if n == 0 {
            saw_eof = true;
            break;
        }
        total_read += n;

        if header_end.is_none() {
            header_end = protocol::find_double_crlf(&buf[..total_read]);
        }

        if let Some(hdr_end) = header_end {
            let body_start = hdr_end + DOUBLE_CRLF_LEN;
            if let Some(content_length) = codec::content_length(&buf[..hdr_end]) {
                if total_read >= body_start.saturating_add(content_length) {
                    request_complete = true;
                    break;
                }
            } else {
                // No Content-Length — headers are complete, no body expected
                request_complete = true;
                break;
            }
        }
    }

    if request_complete {
        return Ok(total_read);
    }
    if saw_eof {
        return Err(Error::InvalidResponse("Incomplete request"));
    }
    Err(Error::BufferOverflow)
}

fn text_error<const MAX_RESPONSE_SIZE: usize>(
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

    #[test]
    fn test_incomplete_request_body_is_rejected() {
        // Declares Content-Length: 100 but the peer closes after 5 body bytes.
        let request =
            b"POST /echo HTTP/1.1\r\nHost: example.com\r\nContent-Length: 100\r\n\r\nhello";
        let mut stream = MockStream::<128, 512>::new(request);
        let mut handler = BodyCheckHandler;

        let err = futures_lite::future::block_on(
            handle_http_connection_with_sizes::<_, _, 128, 512>(&mut stream, &mut handler),
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
        assert!(stream.output.is_empty());
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

        assert!(matches!(err, Error::InvalidResponse("Incomplete request")));
    }
}
