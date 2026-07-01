use crate::{
    error::Error,
    handler::HttpHandler,
    header::mime_types,
    request::HttpRequest,
    response::{HttpResponse, HttpResponseBuilder},
    server::{ServerTimeouts, handle_http_connection_with_sizes},
    status_code::StatusCode,
};
use embassy_net::{Stack, tcp::TcpSocket};
use embassy_time::{Duration, Timer, with_timeout};

const SERVER_BUFFER_SIZE: usize = 4096;
const MAX_REQUEST_SIZE: usize = 4096;
const DEFAULT_MAX_RESPONSE_SIZE: usize = 4096;

/// Simple HTTP server implementation
///
/// **Note**: This server only supports HTTP connections, not HTTPS/TLS.
/// For secure connections, consider using a reverse proxy or load balancer
/// that handles TLS termination.
pub struct EmbassyHttpServer<
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
> EmbassyHttpServer<RX_SIZE, TX_SIZE, REQ_SIZE, MAX_RESPONSE_SIZE>
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
    pub async fn serve<H>(&mut self, stack: Stack<'_>, mut handler: H) -> !
    where
        H: HttpHandler,
    {
        info!("HTTP server started on port {}", self.port);

        let mut rx_buffer = [0; RX_SIZE];
        let mut tx_buffer = [0; TX_SIZE];
        loop {
            let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
            socket.set_timeout(Some(Duration::from_secs(self.timeouts.accept_timeout)));

            if let Err(e) = socket.accept(self.port).await {
                warn!("Accept error: {:?}", e);
                Timer::after(Duration::from_millis(100)).await;
                continue;
            }

            socket.set_timeout(Some(Duration::from_secs(self.timeouts.read_timeout)));

            let mut handler = TimeoutHandler {
                inner: &mut handler,
                timeout_secs: self.timeouts.handler_timeout,
            };

            if let Err(e) = handle_http_connection_with_sizes::<_, _, REQ_SIZE, MAX_RESPONSE_SIZE>(
                &mut socket,
                &mut handler,
            )
            .await
            {
                error!("Error handling request: {:?}", e);
            }

            socket.close();
        }
    }
}

struct TimeoutHandler<'a, H> {
    inner: &'a mut H,
    timeout_secs: u64,
}

impl<H> HttpHandler for TimeoutHandler<'_, H>
where
    H: HttpHandler,
{
    async fn handle_request(
        &mut self,
        request: &HttpRequest<'_>,
    ) -> Result<HttpResponse<'_>, Error> {
        match with_timeout(
            Duration::from_secs(self.timeout_secs),
            self.inner.handle_request(request),
        )
        .await
        {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(e)) => {
                warn!("Handler error: {:?}", e);
                Err(e)
            }
            Err(_) => {
                warn!("Request handling timed out");
                HttpResponseBuilder::new()
                    .status(StatusCode::RequestTimeout)
                    .content_type(mime_types::TEXT)?
                    .text("Request Timeout")
                    .build()
            }
        }
    }
}

/// Type alias for `EmbassyHttpServer` with default buffer sizes (4KB each)
pub type DefaultEmbassyHttpServer = EmbassyHttpServer<
    SERVER_BUFFER_SIZE,
    SERVER_BUFFER_SIZE,
    MAX_REQUEST_SIZE,
    DEFAULT_MAX_RESPONSE_SIZE,
>;

/// Type alias for `EmbassyHttpServer` with small buffer sizes for memory-constrained environments (1KB each)
pub type SmallEmbassyHttpServer = EmbassyHttpServer<1024, 1024, 1024, 1024>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_server_creation() {
        let server: DefaultEmbassyHttpServer = EmbassyHttpServer::new(8080);
        assert_eq!(server.port, 8080);
        assert_eq!(server.timeouts.accept_timeout, 10);
        assert_eq!(server.timeouts.read_timeout, 30);
        assert_eq!(server.timeouts.handler_timeout, 60);

        let server: SmallEmbassyHttpServer = EmbassyHttpServer::new(3000);
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
        let server =
            EmbassyHttpServer::<1024, 1024, 1024, 1024>::with_timeouts(8080, custom_timeouts);
        assert_eq!(server.port, 8080);
        assert_eq!(server.timeouts.accept_timeout, 5);
        assert_eq!(server.timeouts.read_timeout, 15);
        assert_eq!(server.timeouts.handler_timeout, 45);
    }
}
