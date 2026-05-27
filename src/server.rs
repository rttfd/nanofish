use crate::{
    error::Error,
    handler::HttpHandler,
    header::{HttpHeader, headers::CONTENT_LENGTH, mime_types},
    protocol::{self, DOUBLE_CRLF_LEN},
    request::HttpRequest,
    response::{HttpResponse, ResponseBody},
    status_code::StatusCode,
};
use embassy_net::{Stack, tcp::TcpSocket};
use embassy_time::{Duration, Timer, with_timeout};
use embedded_io_async::Write as EmbeddedWrite;
use heapless::Vec;

const SERVER_BUFFER_SIZE: usize = 4096;
const MAX_REQUEST_SIZE: usize = 4096;
const DEFAULT_MAX_RESPONSE_SIZE: usize = 4096;

/// HTTP server timeout configuration
#[derive(Debug, Clone, Copy)]
pub struct ServerTimeouts {
    /// Socket accept timeout in seconds
    pub accept_timeout: u64,
    /// Socket read timeout in seconds
    pub read_timeout: u64,
    /// Request handler timeout in seconds
    pub handler_timeout: u64,
}

impl Default for ServerTimeouts {
    fn default() -> Self {
        Self {
            accept_timeout: 10,
            read_timeout: 30,
            handler_timeout: 60,
        }
    }
}

impl ServerTimeouts {
    /// Create new server timeouts with custom values
    #[must_use]
    pub const fn new(accept_timeout: u64, read_timeout: u64, handler_timeout: u64) -> Self {
        Self {
            accept_timeout,
            read_timeout,
            handler_timeout,
        }
    }
}

/// Simple HTTP server implementation
///
/// **Note**: This server only supports HTTP connections, not HTTPS/TLS.
/// For secure connections, consider using a reverse proxy or load balancer
/// that handles TLS termination.
pub struct HttpServer<
    const RX_SIZE: usize,
    const TX_SIZE: usize,
    const REQ_SIZE: usize,
    const MAX_RESPONSE_SIZE: usize,
> {
    port: u16,
    timeouts: ServerTimeouts,
}

impl<
    const RX_SIZE: usize,
    const TX_SIZE: usize,
    const REQ_SIZE: usize,
    const MAX_RESPONSE_SIZE: usize,
> HttpServer<RX_SIZE, TX_SIZE, REQ_SIZE, MAX_RESPONSE_SIZE>
{
    /// Create a new HTTP server with default timeouts
    #[must_use]
    pub fn new(port: u16) -> Self {
        Self {
            port,
            timeouts: ServerTimeouts::default(),
        }
    }

    /// Create a new HTTP server with custom timeouts
    #[must_use]
    pub const fn with_timeouts(port: u16, timeouts: ServerTimeouts) -> Self {
        Self { port, timeouts }
    }

    /// Start the HTTP server and handle incoming connections
    ///
    /// **Important**: This server only accepts plain HTTP connections.
    /// HTTPS/TLS is not supported by the server (only by the client).
    #[expect(clippy::future_not_send)]
    pub async fn serve<H>(&mut self, stack: Stack<'_>, handler: H) -> !
    where
        H: HttpHandler,
    {
        info!("HTTP server started on port {}", self.port);

        let mut rx_buffer = [0; RX_SIZE];
        let mut tx_buffer = [0; TX_SIZE];
        let mut buf = [0; REQ_SIZE];

        loop {
            let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
            socket.set_timeout(Some(Duration::from_secs(self.timeouts.accept_timeout)));

            if let Err(e) = socket.accept(self.port).await {
                warn!("Accept error: {:?}", e);
                Timer::after(Duration::from_millis(100)).await;
                continue;
            }

            // Reset socket timeout after accept so it doesn't race with read timeout
            socket.set_timeout(None);

            // Read loop: accumulate data until headers + body are complete
            let mut total_read = 0;
            let read_ok = match with_timeout(
                Duration::from_secs(self.timeouts.read_timeout),
                Self::read_request(&mut socket, &mut buf, &mut total_read),
            )
            .await
            {
                Ok(Ok(())) => true,
                Ok(Err(e)) => {
                    warn!("Read error: {:?}", e);
                    false
                }
                Err(_) => {
                    warn!("Socket read timeout");
                    false
                }
            };

            if !read_ok || total_read == 0 {
                socket.close();
                continue;
            }

            // Parse the request
            match self.handle_connection(&buf[..total_read], &handler).await {
                Ok(response_bytes) => {
                    if let Err(e) = socket.write_all(&response_bytes).await {
                        warn!("Failed to write response: {:?}", e);
                    }
                    if let Err(e) = socket.flush().await {
                        warn!("Failed to flush response: {:?}", e);
                    }
                }
                Err(e) => {
                    error!("Error handling request: {:?}", e);
                    if let Ok(error_bytes) = Self::text_error_response(
                        StatusCode::InternalServerError,
                        "Internal Server Error",
                    ) {
                        let _ = socket.write_all(&error_bytes).await;
                        let _ = socket.flush().await;
                    }
                }
            }

            socket.close();
        }
    }

    /// Read a complete HTTP request from the socket.
    ///
    /// Accumulates data until headers are found (`\r\n\r\n`), then reads
    /// any remaining body bytes indicated by `Content-Length`.
    #[expect(clippy::future_not_send)]
    async fn read_request(
        socket: &mut TcpSocket<'_>,
        buf: &mut [u8],
        total_read: &mut usize,
    ) -> Result<(), embassy_net::tcp::Error> {
        let mut header_end = None;

        while *total_read < buf.len() {
            let n = socket.read(&mut buf[*total_read..]).await?;
            if n == 0 {
                break;
            }
            *total_read += n;

            // Look for end of headers if not yet found
            if header_end.is_none() {
                header_end = protocol::find_double_crlf(&buf[..*total_read]);
            }

            if let Some(hdr_end) = header_end {
                let body_start = hdr_end + DOUBLE_CRLF_LEN;
                // Try to determine Content-Length from the headers
                if let Some(cl) = Self::parse_content_length(&buf[..hdr_end]) {
                    if *total_read >= body_start + cl {
                        break;
                    }
                } else {
                    // No Content-Length — headers are complete, no body expected
                    break;
                }
            }
        }
        Ok(())
    }

    /// Extract `Content-Length` value from raw header bytes.
    fn parse_content_length(header_bytes: &[u8]) -> Option<usize> {
        let headers_str = core::str::from_utf8(header_bytes).ok()?;
        protocol::find_header_value(headers_str, CONTENT_LENGTH)?
            .parse()
            .ok()
    }

    /// Build a plain-text error response.
    fn text_error_response(
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

    #[expect(clippy::future_not_send)]
    async fn handle_connection<H>(
        &self,
        buffer: &[u8],
        handler: &H,
    ) -> Result<Vec<u8, MAX_RESPONSE_SIZE>, Error>
    where
        H: HttpHandler,
    {
        // Parse the request
        let request = HttpRequest::try_from(buffer)?;

        // Handle the request
        let response = match with_timeout(
            Duration::from_secs(self.timeouts.handler_timeout),
            handler.handle_request(&request),
        )
        .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(e)) => {
                warn!("Handler error: {:?}", e);
                return Self::text_error_response(
                    StatusCode::InternalServerError,
                    "Internal Server Error",
                );
            }
            Err(_) => {
                warn!("Request handling timed out");
                return Self::text_error_response(StatusCode::RequestTimeout, "Request Timeout");
            }
        };

        response.build_bytes::<MAX_RESPONSE_SIZE>()
    }
}

/// Type alias for `HttpServer` with default buffer sizes (4KB each)
pub type DefaultHttpServer =
    HttpServer<SERVER_BUFFER_SIZE, SERVER_BUFFER_SIZE, MAX_REQUEST_SIZE, DEFAULT_MAX_RESPONSE_SIZE>;

/// Type alias for `HttpServer` with small buffer sizes for memory-constrained environments (1KB each)
pub type SmallHttpServer = HttpServer<1024, 1024, 1024, 1024>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_server_creation() {
        let server: DefaultHttpServer = HttpServer::new(8080);
        assert_eq!(server.port, 8080);
        assert_eq!(server.timeouts.accept_timeout, 10);
        assert_eq!(server.timeouts.read_timeout, 30);
        assert_eq!(server.timeouts.handler_timeout, 60);

        let server: SmallHttpServer = HttpServer::new(3000);
        assert_eq!(server.port, 3000);
    }

    #[test]
    fn test_server_timeouts() {
        // Test default timeouts
        let timeouts = ServerTimeouts::default();
        assert_eq!(timeouts.accept_timeout, 10);
        assert_eq!(timeouts.read_timeout, 30);
        assert_eq!(timeouts.handler_timeout, 60);

        // Test custom timeouts
        let custom_timeouts = ServerTimeouts::new(5, 15, 45);
        assert_eq!(custom_timeouts.accept_timeout, 5);
        assert_eq!(custom_timeouts.read_timeout, 15);
        assert_eq!(custom_timeouts.handler_timeout, 45);

        // Test server with custom timeouts
        let server = HttpServer::<1024, 1024, 1024, 1024>::with_timeouts(8080, custom_timeouts);
        assert_eq!(server.port, 8080);
        assert_eq!(server.timeouts.accept_timeout, 5);
        assert_eq!(server.timeouts.read_timeout, 15);
        assert_eq!(server.timeouts.handler_timeout, 45);
    }
}
