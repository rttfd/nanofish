use crate::{
    error::Error,
    handler::HttpHandler,
    header::mime_types,
    options::TimeoutDuration,
    request::HttpRequest,
    response::{HttpResponse, HttpResponseBuilder},
    server::handle_http_connection_with_sizes,
    status_code::StatusCode,
};
use embassy_net::{Stack, tcp::TcpSocket};
use embassy_time::{Duration, Timer, with_timeout};

const SERVER_BUFFER_SIZE: usize = 4096;
const MAX_REQUEST_SIZE: usize = 4096;
const DEFAULT_MAX_RESPONSE_SIZE: usize = 4096;

/// HTTP server timeout configuration for the Embassy-backed server.
#[derive(Debug, Clone, Copy)]
pub struct ServerTimeouts {
    /// Maximum time to wait for accepting a connection.
    pub accept_timeout: TimeoutDuration,
    /// Maximum inactivity period on the socket while a connection is being
    /// handled: the connection is aborted when no traffic is seen for this
    /// long between socket reads and writes.
    pub read_timeout: TimeoutDuration,
    /// Maximum time a request handler may run per request.
    pub handler_timeout: TimeoutDuration,
}

impl Default for ServerTimeouts {
    fn default() -> Self {
        Self {
            accept_timeout: TimeoutDuration::from_secs(10),
            read_timeout: TimeoutDuration::from_secs(30),
            handler_timeout: TimeoutDuration::from_secs(60),
        }
    }
}

impl ServerTimeouts {
    /// Create new server timeouts with custom values.
    #[must_use]
    pub const fn new(
        accept_timeout: TimeoutDuration,
        read_timeout: TimeoutDuration,
        handler_timeout: TimeoutDuration,
    ) -> Self {
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
            socket.set_timeout(Some(Duration::from_millis(
                self.timeouts.accept_timeout.as_millis(),
            )));

            if let Err(e) = socket.accept(self.port).await {
                warn!("Accept error: {:?}", e);
                Timer::after(Duration::from_millis(100)).await;
                continue;
            }

            // Abort the connection when it goes idle, so half-open peers
            // cannot hold buffers forever.
            socket.set_timeout(Some(Duration::from_millis(
                self.timeouts.read_timeout.as_millis(),
            )));

            let mut handler = TimeoutHandler {
                inner: &mut handler,
                timeout: self.timeouts.handler_timeout,
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
    timeout: TimeoutDuration,
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
            Duration::from_millis(self.timeout.as_millis()),
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
        assert_eq!(
            server.timeouts.accept_timeout,
            TimeoutDuration::from_secs(10)
        );
        assert_eq!(server.timeouts.read_timeout, TimeoutDuration::from_secs(30));
        assert_eq!(
            server.timeouts.handler_timeout,
            TimeoutDuration::from_secs(60)
        );

        let server: SmallEmbassyHttpServer = EmbassyHttpServer::new(3000);
        assert_eq!(server.port, 3000);
    }

    #[test]
    fn test_server_timeouts() {
        // Test default timeouts
        let timeouts = ServerTimeouts::default();
        assert_eq!(timeouts.accept_timeout, TimeoutDuration::from_secs(10));
        assert_eq!(timeouts.read_timeout, TimeoutDuration::from_secs(30));
        assert_eq!(timeouts.handler_timeout, TimeoutDuration::from_secs(60));

        // Test custom timeouts
        let custom_timeouts = ServerTimeouts::new(
            TimeoutDuration::from_secs(5),
            TimeoutDuration::from_secs(15),
            TimeoutDuration::from_secs(45),
        );
        assert_eq!(
            custom_timeouts.accept_timeout,
            TimeoutDuration::from_secs(5)
        );
        assert_eq!(custom_timeouts.read_timeout, TimeoutDuration::from_secs(15));
        assert_eq!(
            custom_timeouts.handler_timeout,
            TimeoutDuration::from_secs(45)
        );

        // Test server with custom timeouts
        let server =
            EmbassyHttpServer::<1024, 1024, 1024, 1024>::with_timeouts(8080, custom_timeouts);
        assert_eq!(server.port, 8080);
        assert_eq!(
            server.timeouts.accept_timeout,
            TimeoutDuration::from_secs(5)
        );
        assert_eq!(server.timeouts.read_timeout, TimeoutDuration::from_secs(15));
        assert_eq!(
            server.timeouts.handler_timeout,
            TimeoutDuration::from_secs(45)
        );
    }
}
