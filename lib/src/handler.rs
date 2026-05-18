pub use crate::abstarct_socket::socket::SocketWrite as HttpSocketWrite;
use crate::request::HttpRequest;

use crate::allocator::HttpAllocator;
use crate::error::Error;
use crate::response::HttpResponse;

#[cfg(feature = "ws")]
pub use crate::abstarct_socket::socket::SocketStream as WebSocketStream;

/// The WebSocket implementation
#[cfg(feature = "ws")]
pub type WebSocket<'a, Socket> = crate::web_socket::WebSocket<'a, Socket>;

#[cfg(feature = "ws")]
pub use crate::web_socket::{WebSocketError, WebSocketState};

#[cfg(feature = "ws")]
pub use crate::web_socket::{WebSocketIoError, WebSocketRead, WebSocketReadReady, WebSocketWrite, WebSocketWriteReady};

/// Trait for handling incoming HTTP requests.
/// Implementers of this trait can define custom logic to process HTTP requests and generate appropriate responses.
#[allow(async_fn_in_trait)]
pub trait HttpHandler {
    /// Handle an incoming HTTP request and produce a response.
    /// The handler is responsible for writing the response to the provided `http_socket`.
    /// The `context_id` can be used to track the current worker context for the request, if needed.
    /// The handler should return an `HttpResponse` on success, or an `Error` if the request could not be processed.
    /// The handler can also use the `allocator` to manage memory for the request processing, if needed.
    ///
    /// Note: The handler should not close the `http_socket` after writing the response, as the server will manage the socket lifecycle.
    /// The handler should also ensure that the response is fully written to the socket before returning, as the server may reuse the socket for subsequent requests.
    ///
    /// ### Arguments:
    /// * `allocator` - A mutable reference to an `HttpAllocator` for managing memory
    /// * `request` - A reference to the incoming `HttpRequest`
    /// * `http_socket` - A mutable reference to the socket for writing the response
    /// * `context_id` - An identifier for the request context
    ///
    /// ### Returns:
    /// * `Ok(HttpResponse)` - The response to be sent back to the client
    /// * `Err(Error)` - An error indicating that the request could not be processed
    ///
    async fn handle_request(
        &mut self,
        allocator: &mut HttpAllocator<'_>,
        request: &HttpRequest<'_>,
        http_socket: &mut impl HttpSocketWrite,
        context_id: usize,
    ) -> Result<HttpResponse, Error>;

    #[cfg(feature = "ws")]
    /// Handle a WebSocket connection
    ///
    /// If the handler returns result Ok() the WebSocket connection will be automatically closed, but the TCP socket
    /// will remain open and server will process further HTTP requests on it.
    ///
    /// If the handler returns Err() the TCP socket will be closed and the server will wait for a new connection.
    async fn handle_websocket_connection(
        &mut self,
        _request: &HttpRequest<'_>,
        _web_socket: &mut impl WebSocketStream,
        _context_id: usize,
    ) -> Result<(), ()> {
        //By default, any incoming WebSocket connection will be silently closed.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //TODO: add tests for HttpHandler implementations
}
