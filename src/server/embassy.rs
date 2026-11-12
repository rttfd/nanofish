use embassy_net::{
    Stack,
    tcp::{State, TcpSocket},
};
use embassy_time::{Duration, Timer, with_timeout};
use embedded_io_async::{Read, Write};
use nanofish::{
    error::Error,
    handler::HttpHandler,
    header::{headers::CONNECTION, mime_types},
    options::TimeoutDuration,
    request::HttpRequest,
    server::response::{ServerResponse, ServerResponseBuilder},
    server::{
        RequestBody, StreamingHandler, handle_http_connection_with_sizes,
        handle_http_stream_connection_with_sizes, serve_http_connection_with_sizes,
    },
    status_code::StatusCode,
};

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
    /// Maximum time a request handler may run per request. A handler that
    /// takes longer is abandoned and the client gets `503 Service
    /// Unavailable`.
    ///
    /// The time a handler spends reading the request body counts, so a slow
    /// upload needs a longer timeout than the handler's own work does.
    pub handler_timeout: TimeoutDuration,
    /// Maximum total time one connection may take, from accept to the end of
    /// the response.
    ///
    /// `read_timeout` only fires on inactivity, so a client that sends one
    /// byte just under that interval can otherwise keep a connection (and the
    /// server's single socket) busy indefinitely.
    pub connection_timeout: TimeoutDuration,
}

impl Default for ServerTimeouts {
    fn default() -> Self {
        Self {
            accept_timeout: TimeoutDuration::from_secs(10),
            read_timeout: TimeoutDuration::from_secs(30),
            handler_timeout: TimeoutDuration::from_secs(60),
            connection_timeout: TimeoutDuration::from_secs(120),
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
        connection_timeout: TimeoutDuration,
    ) -> Self {
        Self {
            accept_timeout,
            read_timeout,
            handler_timeout,
            connection_timeout,
        }
    }
}

/// Simple HTTP server implementation
///
/// **Note**: This adapter accepts plain TCP; Nanofish does not terminate TLS.
/// For HTTPS, run [`handle_http_connection`](nanofish::handle_http_connection)
/// over a TLS stream you bring, or put a reverse proxy or load balancer that
/// handles TLS termination in front of the device.
pub struct EmbassyHttpServer<
    const RX_SIZE: usize,
    const TX_SIZE: usize,
    const REQ_SIZE: usize,
    const MAX_RESPONSE_SIZE: usize,
> {
    port: u16,
    timeouts: ServerTimeouts,
    /// Requests served per connection; `1` closes after every response.
    max_requests: usize,
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
            max_requests: 1,
        }
    }

    /// Create a new HTTP server with custom timeouts
    #[must_use]
    pub const fn with_timeouts(port: u16, timeouts: ServerTimeouts) -> Self {
        Self {
            port,
            timeouts,
            max_requests: 1,
        }
    }

    /// Keep each connection open for up to `max_requests` requests (HTTP/1.1
    /// keep-alive) instead of closing it after every response. `0` and `1`
    /// both mean one request per connection, which is the default.
    ///
    /// A client that is done but keeps the connection open occupies the
    /// server's socket until [`ServerTimeouts::read_timeout`] expires, so
    /// lower that timeout when enabling this, or run several server tasks.
    /// Applies to [`EmbassyHttpServer::serve`]; streamed responses always
    /// close the connection.
    #[must_use]
    pub const fn with_keep_alive(mut self, max_requests: usize) -> Self {
        self.max_requests = max_requests;
        self
    }

    /// Start the HTTP server and handle incoming connections
    ///
    /// **Important**: This server only accepts plain TCP connections; the
    /// `tls` feature covers the client only. See the type's docs for the
    /// options to serve HTTPS.
    ///
    /// Connections are handled one at a time on a single socket, and there is
    /// no listen backlog: while every task is busy, a new connection is
    /// refused. To serve several clients at once, run this method in several
    /// tasks (a browser opens about six connections per page, so run about
    /// six): each task owns its socket and buffers, and all of them accept on
    /// the same port.
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
    pub async fn serve<H>(&mut self, stack: Stack<'_>, mut handler: H) -> !
    where
        H: HttpHandler,
    {
        let handler_timeout = self.timeouts.handler_timeout;
        let max_requests = self.max_requests;
        self.serve_connections(stack, async |socket| {
            let mut handler = TimeoutHandler {
                inner: &mut handler,
                timeout: handler_timeout,
            };
            if max_requests > 1 {
                serve_http_connection_with_sizes::<_, _, REQ_SIZE, MAX_RESPONSE_SIZE>(
                    socket,
                    &mut handler,
                    max_requests,
                )
                .await
            } else {
                handle_http_connection_with_sizes::<_, _, REQ_SIZE, MAX_RESPONSE_SIZE>(
                    socket,
                    &mut handler,
                )
                .await
            }
        })
        .await
    }

    /// Like [`EmbassyHttpServer::serve`], for a [`StreamingHandler`] that
    /// sends its responses in pieces.
    ///
    /// `MAX_RESPONSE_SIZE` bounds the status line and headers of each
    /// response; the body is not limited by it. `ServerTimeouts::handler_timeout`
    /// does not apply, since a streamed response can legitimately take long;
    /// `ServerTimeouts::connection_timeout` bounds the whole connection.
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
    pub async fn serve_streaming<H>(&mut self, stack: Stack<'_>, mut handler: H) -> !
    where
        H: StreamingHandler,
    {
        self.serve_connections(stack, async |socket| {
            handle_http_stream_connection_with_sizes::<_, _, REQ_SIZE, MAX_RESPONSE_SIZE>(
                socket,
                &mut handler,
            )
            .await
        })
        .await
    }

    /// Accept connections forever, running `connection` on each.
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
    async fn serve_connections(
        &self,
        stack: Stack<'_>,
        mut connection: impl AsyncFnMut(&mut TcpSocket<'_>) -> Result<(), Error>,
    ) -> ! {
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

            match with_timeout(
                Duration::from_millis(self.timeouts.connection_timeout.as_millis()),
                connection(&mut socket),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(e)) => error!("Error handling request: {:?}", e),
                Err(_) => {
                    warn!("Connection timed out");
                    socket.abort();
                    let _ = with_timeout(CLOSE_GRACE, socket.flush()).await;
                }
            }

            finish_socket(&mut socket).await;
        }
    }
}

/// How long [`finish_socket`] waits for the peer to acknowledge our FIN.
const CLOSE_GRACE: Duration = Duration::from_secs(2);

/// How often [`finish_socket`] looks at the socket state while it waits.
const CLOSE_POLL: Duration = Duration::from_millis(10);

/// Whether a socket is still waiting for the peer to acknowledge our FIN.
const fn is_closing(state: State) -> bool {
    matches!(state, State::FinWait1 | State::Closing | State::LastAck)
}

/// Close `socket` and give the stack time to put the FIN on the wire.
///
/// `TcpSocket::close` only queues the FIN; the network task sends it on its
/// next poll. Dropping the socket (or reusing its buffers for the next
/// `accept`) right after `close` removes it from the stack before that poll,
/// so the peer never sees the end of the stream and the late FIN from the
/// client is answered with an RST. Waiting here keeps the response and the
/// FIN in order. If the peer stays silent past [`CLOSE_GRACE`], the socket is
/// aborted so a dead client cannot hold the listener.
#[expect(
    clippy::future_not_send,
    reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
)]
async fn finish_socket(socket: &mut TcpSocket<'_>) {
    socket.close();
    let _ = with_timeout(CLOSE_GRACE, async {
        // Wait for unsent data and the FIN to go out, then for the FIN's ACK.
        let _ = socket.flush().await;
        while is_closing(socket.state()) {
            Timer::after(CLOSE_POLL).await;
        }
    })
    .await;
    if socket.state() != State::Closed && socket.state() != State::TimeWait {
        socket.abort();
        let _ = with_timeout(CLOSE_GRACE, socket.flush()).await;
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
    async fn handle_request<S: Read + Write>(
        &mut self,
        request: &HttpRequest<'_>,
        body: &mut RequestBody<'_, S>,
    ) -> Result<ServerResponse<'_>, Error> {
        match with_timeout(
            Duration::from_millis(self.timeout.as_millis()),
            self.inner.handle_request(request, body),
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
                // The server, not the client, was too slow: `503`, not
                // `408 Request Timeout`, which blames the client. The
                // connection closes, since the abandoned handler may have
                // left it mid-way.
                ServerResponseBuilder::new()
                    .status(StatusCode::ServiceUnavailable)
                    .content_type(mime_types::TEXT)?
                    .header(CONNECTION, "close")?
                    .text("Service Unavailable")
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
    use nanofish::server::embassy::*;

    #[test]
    fn closing_states_are_the_ones_waiting_for_our_fin_to_be_acked() {
        use embassy_net::tcp::State;

        for state in [State::FinWait1, State::Closing, State::LastAck] {
            assert!(is_closing(state), "{state:?} still owes an ACK");
        }
        for state in [
            State::Closed,
            State::TimeWait,
            State::FinWait2,
            State::Established,
        ] {
            assert!(
                !is_closing(state),
                "{state:?} is not waiting for our FIN's ACK"
            );
        }
    }

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
        assert_eq!(
            server.timeouts.connection_timeout,
            TimeoutDuration::from_secs(120)
        );

        let server: SmallEmbassyHttpServer = EmbassyHttpServer::new(3000);
        assert_eq!(server.port, 3000);
        assert_eq!(server.max_requests, 1);
        assert_eq!(server.with_keep_alive(8).max_requests, 8);
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
            TimeoutDuration::from_secs(90),
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
        assert_eq!(
            custom_timeouts.connection_timeout,
            TimeoutDuration::from_secs(90)
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
