use crate::{
    handler::HttpHandler,
    request::HttpRequest,
    response::{HttpResponse, HttpResponseBuilder},
};

use crate::error::Error;
use crate::socket::*;
use crate::status_code::StatusCode;
use core::time::Duration;
use defmt_or_log as log;
use prefix_arena::PrefixArena;

/// Re-exports for easier access by users of the library
pub use crate::socket::{AbstractSocketListener, SocketEndpoint};

/// Re-exports of the memory buffer trait and related types for easier allocation of memory for HTTP request handling.
pub use crate::worker_memory::*;

// WebSocket related imports and constants
#[cfg(feature = "ws")]
use crate::socket::SocketWrite;
#[cfg(feature = "ws")]
use crate::web_socket::WebSocket;
#[cfg(feature = "ws")]
use sha1::{Digest, Sha1};
#[cfg(feature = "ws")]
const WS_GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

const READ_TIMEOUT_SECS: u64 = 30;
const HANDLER_TIMEOUT_SECS: u64 = 60;

/// This module contains the embassy implementation of TCP socket pool, that manage multiple TCP socket connections efficiently.
/// It provides abstractions for creating and handling TCP listeners and connections, allowing the server to manage multiple
/// simultaneous client connections in a cooperative manner, leveraging the async capabilities of the embassy framework.
#[cfg(feature = "embassy_impl")]
pub mod socket_pool {
    pub use crate::socket::embassy_impl::tcp_socket_pool::{TcpSocketPool, TcpSocketPoolRunner, TcpSocketPoolState};
}

/// This module contains the tokio implementation of TCP socket pool, that manage multiple TCP socket connections efficiently.
/// It provides abstractions for creating and handling TCP listeners and connections, allowing the server to manage multiple
/// simultaneous client connections in a cooperative manner, leveraging the async capabilities of the tokio framework.
#[cfg(feature = "tokio_impl")]
pub mod socket_listener {
    pub use crate::socket::tokio_impl::socket::TokioTcpListener;
}

/// HTTP server timeout configuration
#[derive(Debug, Clone, Copy)]
pub struct ServerTimeouts {
    /// Socket read timeout in seconds
    pub read_timeout: u64,
    /// Request handler timeout in seconds
    pub handler_timeout: u64,
}

impl Default for ServerTimeouts {
    fn default() -> Self {
        Self {
            read_timeout: READ_TIMEOUT_SECS,
            handler_timeout: HANDLER_TIMEOUT_SECS,
        }
    }
}

impl ServerTimeouts {
    /// Create new server timeouts with custom values
    #[must_use]
    pub fn new(read_timeout: u64, handler_timeout: u64) -> Self {
        Self {
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
pub struct HttpServer<SocketBuilder> {
    socket_builder: SocketBuilder,
    timeouts: ServerTimeouts,
    auto_close_connection: bool,
}

impl<SocketBuilder: AbstractSocketListener> HttpServer<SocketBuilder> {
    /// Create a new HTTP server with default timeouts
    #[must_use]
    pub fn new(socket_builder: SocketBuilder, timeouts: ServerTimeouts) -> Self {
        Self {
            socket_builder,
            timeouts,
            auto_close_connection: false,
        }
    }

    /// Determine whether to automatically close the connection after handling a request.
    #[must_use]
    pub fn with_auto_close_connection(mut self, auto_close: bool) -> Self {
        // Currently no-op, placeholder for future functionality
        self.auto_close_connection = auto_close;
        self
    }

    /// Start the HTTP server and handle incoming connections
    ///
    /// **Important**: This server only accepts plain HTTP connections.
    /// HTTPS/TLS is not supported by the server (only by the client).
    pub async fn serve<H>(&self, mut worker_memory: impl HttpMemoryBuffer, mut handler: H, context_id: usize) -> !
    where
        H: HttpHandler,
        <SocketBuilder as AbstractSocketListener>::Socket: AbstractSocket + SocketReadWith,
    {
        log::info!("WebServer[{}]: HTTP server started", context_id);

        log::debug!(
            "WebServer[{}]: Auto-close connection is {}",
            context_id,
            self.auto_close_connection
        );

        loop {
            log::info!("WebServer[{}]: Waiting for new connection...", context_id);

            // Create arena allocator for this connection's request and response processing
            let mut head_arena_alloc = PrefixArena::from_uninit(worker_memory.get_buffer());
            let mut socket = self.socket_builder.accept().await;

            log::info!(
                "WebServer[{}]: New connection/request {:?}",
                context_id,
                socket.remote_endpoint()
            );

            let request = match with_timeout(
                Duration::from_secs(self.timeouts.read_timeout),
                HttpRequest::try_parse_from_stream(&mut socket, &mut head_arena_alloc),
            )
            .await
            {
                Ok(Ok(request)) => request,
                Ok(Err(e)) => {
                    log::warn!(
                        "WebServer[{}]: Read error: {:?}, {:?}",
                        context_id,
                        e,
                        socket.remote_endpoint()
                    );
                    self.close_connection(socket, context_id).await;
                    continue;
                }
                Err(_) => {
                    log::warn!(
                        "WebServer[{}]: Socket read timeout, {:?}",
                        context_id,
                        socket.remote_endpoint()
                    );
                    self.close_connection(socket, context_id).await;
                    continue;
                }
            };

            #[cfg(feature = "ws")]
            // Check if the request is a WebSocket upgrade request
            if let Some(web_socket_key) = request.web_socket_key {
                log::info!(
                    "WebServer[{}]: Process the websocket connection from, {:?}",
                    context_id,
                    socket.remote_endpoint()
                );
                if self
                    .web_socket_handshake(&mut head_arena_alloc, web_socket_key, &mut socket, context_id)
                    .await
                    .is_err()
                {
                    // Handshake failed, close the connection
                    self.close_connection(socket, context_id).await;
                    continue;
                }

                //let socket_ref: &mut Socket = &mut socket;
                let mut web_socket = WebSocket::new(&mut socket);
                if let Err(e) = handler
                    .handle_websocket_connection(&request, &mut web_socket, context_id)
                    .await
                {
                    // Handle error during WebSocket connection
                    log::error!(
                        "WebServer[{}]: Error handling WebSocket connection: {:?}",
                        context_id,
                        e
                    );
                }

                // Ensure the WebSocket connection is closed gracefully
                if let Err(e) = web_socket.close().await {
                    log::error!("WebServer[{}]: Error closing WebSocket connection: {}", context_id, e);
                }
                // After handling the WebSocket connection, we will close the TCP connection and wait for a new one
                self.close_connection(socket, context_id).await;
                continue;
            }
            // For regular HTTP requests, we will process them as usual
            {
                log::info!(
                    "WebServer[{}]: Process the request of, {:?}",
                    context_id,
                    socket.remote_endpoint()
                );

                match self
                    .handle_connection(&mut head_arena_alloc, &request, &mut socket, &mut handler, context_id)
                    .await
                {
                    Ok(_) => {
                        log::info!(
                            "WebServer[{}]: Request handled successfully, {:?}",
                            context_id,
                            socket.remote_endpoint()
                        );
                    }
                    Err(e) => {
                        log::error!("WebServer[{}]: Error handling request: {:?}", context_id, e);
                        // Send a 500 error response
                        if self.send_server_internal_error(&mut socket, context_id).await.is_err() {
                            // Failed to send error response, close the connection
                            log::error!(
                                "WebServer[{}]: Failed to send internal server error response",
                                context_id
                            );
                        }
                        self.close_connection(socket, context_id).await;
                        continue;
                    }
                }
            }

            log::debug!(
                "WebServer[{}]: It is about to process following request... {:?}",
                context_id,
                socket.remote_endpoint()
            );
        }
    }

    #[cfg(feature = "ws")]
    async fn web_socket_handshake<Socket: SocketWrite>(
        &self,
        allocator: &mut PrefixArena<'_>,
        web_socket_key: &str,
        tcp_socket: &mut Socket,
        context_id: usize,
    ) -> Result<(), ()> {
        log::info!("WebServer[{}]: WebSocket upgrade request detected", context_id);
        let res = try_handle_websocket_handshake(allocator, tcp_socket, web_socket_key).await;

        match res {
            Ok(()) => {
                log::info!("WebServer[{}]: WebSocket handshake successful", context_id);
                Ok(())
            }
            Err(e) => {
                log::error!("WebServer[{}]: WebSocket handshake error: {:?}", context_id, e);
                // Send a 500 error response
                self.send_server_internal_error(tcp_socket, context_id).await
            }
        }
    }

    async fn send_response<Socket: SocketWrite>(
        &self,
        socket: &mut Socket,
        response_bytes: &[u8],
        context_id: usize,
    ) -> Result<(), ()> {
        #[cfg(any(feature = "defmt", feature = "log"))]
        if response_bytes.len() < 256 {
            log::trace!(
                "WebServer[{}]: Raw response: {:?}",
                context_id,
                core::str::from_utf8(&response_bytes[..response_bytes.len()]).unwrap_or("<invalid utf8>")
            );
        } else {
            log::trace!(
                "WebServer[{}]: Response length: {} bytes",
                context_id,
                response_bytes.len()
            );
        }

        socket.write_all(response_bytes).await.map_err(|e| {
            log::warn!(
                "WebServer[{}]: Failed to write response: {:?}",
                context_id,
                log::Debug2Format(&e)
            );
        })
    }

    async fn send_server_internal_error<Socket: SocketWrite>(
        &self,
        socket: &mut Socket,
        context_id: usize,
    ) -> Result<(), ()> {
        let error_response = b"HTTP/1.1 500 Internal Server Error\r\nContent-Type: text/plain\r\nContent-Length: 21\r\n\r\nInternal Server Error";
        self.send_response(socket, error_response, context_id).await
    }

    /// Close the connection gracefully
    async fn close_connection<Socket: SocketClose + SocketInfo>(&self, mut socket: Socket, context_id: usize) {
        let remote_endpoint = socket.remote_endpoint();

        if socket.close().await.is_err() {
            log::error!("WebServer[{}]: Error while closing connection", context_id);
        }

        log::info!("WebServer[{}]: Connection closed {:?}", context_id, remote_endpoint);
    }

    async fn handle_connection<H, Socket>(
        &self,
        allocator: &mut PrefixArena<'_>,
        request: &HttpRequest<'_>,
        http_socket: &mut Socket,
        handler: &mut H,
        context_id: usize,
    ) -> Result<HttpResponse, Error>
    where
        H: HttpHandler,
        Socket: SocketWrite,
    {
        // Handle the request
        match with_timeout(
            Duration::from_secs(self.timeouts.handler_timeout),
            handler.handle_request(allocator, request, http_socket, context_id),
        )
        .await
        {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(e)) => {
                log::warn!("WebServer[{}]: Handler error: {:?}", context_id, e);

                HttpResponseBuilder::new(http_socket)
                    .with_status(StatusCode::InternalServerError)
                    .await?
                    .with_header("Content-Type", "text/plain")
                    .await?
                    .with_body_from_str("Internal Server Error")
                    .await
            }
            Err(_) => {
                HttpResponseBuilder::new(http_socket)
                    .with_status(StatusCode::InternalServerError)
                    .await?
                    .with_header("Content-Type", "text/plain")
                    .await?
                    .with_body_from_str("Request Timeout")
                    .await
            }
        }
    }
}

/// Handles the WebSocket handshake process.
#[cfg(feature = "ws")]
async fn try_handle_websocket_handshake<Socket>(
    allocator: &mut PrefixArena<'_>,
    http_socket: &mut Socket,
    web_socket_key: &str,
) -> Result<(), Error>
where
    Socket: SocketWrite,
{
    // Compute the Sec-WebSocket-Accept value
    let key_bytes = web_socket_key.as_bytes();
    let mut hasher = Sha1::new();
    hasher.update(key_bytes);
    hasher.update(WS_GUID);
    let hash = hasher.finalize();

    let mut tmp_buf = allocator.view();
    let buf = unsafe { tmp_buf.as_slice_mut_unchecked() };
    let encoded_hash = binascii::b64encode(&hash, buf).map_err(|e| {
        log::error!(
            "WebSocket handshake: Base64 encoding error: {:?}",
            log::Debug2Format(&e)
        );
        match e {
            binascii::ConvertError::InvalidOutputLength => Error::MemoryOverflow,
            _ => Error::ServerError,
        }
    })?;

    let builder = HttpResponseBuilder::new(http_socket);
    builder
        .with_status(crate::status_code::StatusCode::SwitchingProtocols)
        .await?
        .with_header("Upgrade", "websocket")
        .await?
        .with_header("Connection", "Upgrade")
        .await?
        .with_header_from_slice("Sec-WebSocket-Accept", encoded_hash)
        .await?
        .with_no_body()
        .await?;
    Ok(())
}

// Helper function to wrap a timeout logic around a future, since we want to use the same timeout logic for
// both Tokio and Embassy implementations without duplicating code in the main server logic.
#[inline]
async fn with_timeout<F, T>(_duration: Duration, future: F) -> Result<T, ()>
where
    F: core::future::Future<Output = T>,
{
    #[cfg(feature = "embassy_impl")]
    {
        embassy_time::with_timeout(_duration.try_into().unwrap(), future)
            .await
            .map_err(|_| ())
    }
    #[cfg(feature = "tokio_impl")]
    {
        tokio::time::timeout(_duration, future).await.map_err(|_| ())
    }

    #[cfg(not(any(feature = "tokio_impl", feature = "embassy_impl")))]
    // No timeout mechanism available, just await the future directly (this is not ideal, but allows the code to compile without either feature)
    Ok(future.await)
}

#[cfg(test)]
mod tests {
    //TODO: add tests for HttpServer, including:
    // - Test that the server can accept and handle a simple HTTP request correctly
    // - Test that the server can handle multiple requests sequentially
    // - Test that the server can handle WebSocket upgrade requests correctly (if ws feature is enabled)
    // - Test that the server properly handles timeouts and errors, returning appropriate HTTP responses
    // - Test that the server can handle large requests and responses without crashing or leaking memory
}
