#![allow(async_fn_in_trait)]
use crate::socket::SocketEndpoint;

/// A trait representing a socket connector. This trait provides a method for connecting to a remote
/// socket endpoint and obtaining a socket instance representing the established connection. The
/// `AbstractSocketConnector` trait includes an associated type `Socket` that represents the type of
/// socket produced by the connector, and a method for connecting to a remote endpoint. Implementers
/// of the `AbstractSocketConnector` trait must provide an implementation for the `connect` method,
/// which takes a `SocketEndpoint` as an argument and returns a future that resolves to a `Result`
/// containing either an instance of `Self::Socket` representing the established connection or an
/// error if the connection attempt fails. The `AbstractSocketConnector` trait is designed to be
/// flexible and extensible, allowing for different types of socket connectors to be implemented
/// while still adhering to a common interface for establishing connections to remote endpoints.
///
pub trait AbstractSocketConnector {
    /// The associated type representing the socket produced by the connector.
    type Error;
    /// The associated type representing the socket produced by the connector.
    type Socket;

    /// Connect to a remote socket endpoint and obtain a socket instance representing the established
    /// connection.
    ///
    /// ### Arguments
    /// - `endpoint`: The `SocketEndpoint` representing the remote endpoint to connect to.
    ///
    /// ### Returns
    /// - Returns a future that resolves to a `Result` containing either an instance of `Self::Socket`
    ///   representing the established connection or an error if the connection attempt fails.
    async fn connect(&self, endpoint: SocketEndpoint) -> Result<Self::Socket, Self::Error>;
}

impl<T: ?Sized + AbstractSocketConnector> AbstractSocketConnector for &T {
    type Error = T::Error;
    type Socket = T::Socket;

    #[inline]
    async fn connect(&self, endpoint: SocketEndpoint) -> Result<Self::Socket, Self::Error> {
        T::connect(self, endpoint).await
    }
}
