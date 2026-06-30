use crate::{
    error::Error,
    handler::HttpHandler,
    header::{HttpHeader, headers::CONTENT_LENGTH, mime_types},
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
pub struct HttpIoServer<
    const REQ_SIZE: usize = DEFAULT_REQUEST_SIZE,
    const MAX_RESPONSE_SIZE: usize = DEFAULT_RESPONSE_SIZE,
>;

impl HttpIoServer<DEFAULT_REQUEST_SIZE, DEFAULT_RESPONSE_SIZE> {
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

impl Default for HttpIoServer<DEFAULT_REQUEST_SIZE, DEFAULT_RESPONSE_SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

/// Type alias for `HttpIoServer` with default request and response buffer sizes.
pub type DefaultHttpIoServer = HttpIoServer<DEFAULT_REQUEST_SIZE, DEFAULT_RESPONSE_SIZE>;

/// Type alias for `HttpIoServer` with smaller request and response buffer sizes.
pub type SmallHttpIoServer = HttpIoServer<SMALL_REQUEST_SIZE, SMALL_RESPONSE_SIZE>;

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
    if total_read == 0 {
        return Err(Error::NoResponse);
    }

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
            if let Some(content_length) = content_len(&buf[..hdr_end]) {
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

fn content_len(header_bytes: &[u8]) -> Option<usize> {
    let headers_str = core::str::from_utf8(header_bytes).ok()?;
    protocol::find_header_value(headers_str, CONTENT_LENGTH)?
        .parse()
        .ok()
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
        let server = HttpIoServer::<128, 512>::with_buffer_sizes();

        futures_lite::future::block_on(server.handle_connection(&mut stream, &mut handler))
            .unwrap();

        let response = core::str::from_utf8(&stream.output).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.contains("Hello from nanofish HTTP server"));
    }
}
