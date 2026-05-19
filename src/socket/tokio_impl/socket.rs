pub use crate::socket::tokio_impl::tokio_socket_wrapper::{
    TokioSocketOwnedReadHalfWrapper, TokioSocketOwnedWriteHalfWrapper, TokioSocketReadHalfWrapper, TokioSocketWrapper,
    TokioSocketWriteHalfWrapper,
};

use crate::socket::{SocketConnector, SocketEndpoint, SocketListener};
use defmt_or_log as log;
use tokio::net::TcpListener;

/// Tokio implementation of a socket builder that borrows the TcpSocket for its lifetime.
pub struct TokioTcpListener<'stack> {
    listener: TcpListener,
    _marker: core::marker::PhantomData<&'stack ()>,
}

impl<'stack> TokioTcpListener<'stack> {
    /// Create a new TokioTcpSocketBuilder with the specified endpoint.
    /// The endpoint can be a socket address or a string that can be parsed into one.
    /// The builder will bind to the endpoint and be ready to accept connections.
    /// ### Arguments
    /// * `endpoint` - The socket endpoint to bind to, which can be a SocketEndpoint or a type that can be converted into one.
    /// ### Results
    /// Returns a new instance of TokioTcpSocketBuilder that is bound to the specified endpoint and ready to accept connections.
    /// ### Panics
    /// This function will panic if binding to the endpoint fails, which can happen if the address is invalid or already in use.
    pub async fn new(endpoint: impl Into<SocketEndpoint>) -> Self {
        let addr = endpoint.into();
        let listener = TcpListener::bind(addr).await.unwrap_or_else(|e: std::io::Error| {
            log::panic!("Failed to bind to endpoint: {:?}", e);
        });

        Self {
            listener,
            _marker: core::marker::PhantomData,
        }
    }
}

impl<'stack> SocketListener for TokioTcpListener<'stack> {
    type AcceptedSocket = TokioSocketWrapper;

    async fn accept(&self) -> Self::AcceptedSocket {
        self.listener
            .accept()
            .await
            .inspect_err(|e| log::error!("Failed to accept connection: {:?}", e))
            .map(|(soc, _)| TokioSocketWrapper::new_stream(soc))
            .unwrap()
    }

    async fn try_accept(&self) -> Option<Self::AcceptedSocket> {
        core::future::poll_fn(|cx| {
            if let core::task::Poll::Ready(Ok((socket, _))) = self.listener.poll_accept(cx) {
                core::task::Poll::Ready(Some(TokioSocketWrapper::new_stream(socket)))
            } else {
                core::task::Poll::Ready(None)
            }
        })
        .await
    }

    fn local_endpoint(&self) -> SocketEndpoint {
        self.listener.local_addr().unwrap()
    }
}

/// Tokio implementation of a socket connector that creates new TcpStream connections to remote endpoints.
pub struct TokioTcpSocketConnector;

impl TokioTcpSocketConnector {
    /// Create a new instance of TokioTcpSocketConnector.
    pub const fn new() -> Self {
        Self
    }
}

impl Default for TokioTcpSocketConnector {
    fn default() -> Self {
        Self::new()
    }
}

impl SocketConnector for TokioTcpSocketConnector {
    type ConnectError = std::io::Error;
    type ConnectedSocket = TokioSocketWrapper;

    async fn connect(&self, endpoint: SocketEndpoint) -> Result<Self::ConnectedSocket, Self::ConnectError> {
        let stream = tokio::net::TcpStream::connect(endpoint).await?;
        Ok(TokioSocketWrapper::new_stream(stream))
    }
}
