#![allow(async_fn_in_trait)]

pub use embedded_io_async::{
    Error as SocketError, ErrorKind as SocketErrorKind, ErrorType as SocketErrorType, Read as SocketRead,
    ReadExactError as SocketReadExactError, ReadReady as SocketReadReady, Write as SocketWrite,
    WriteReady as SocketWriteReady,
};

/// Trait representing a read stream interface
pub trait SocketReadWith: SocketErrorType {
    /// Read from the stream using the provided function
    ///
    /// The function `f` is called with a slice of available data from the stream.
    /// It should return a tuple containing the number of bytes read and a result value.
    ///
    /// ## Returns
    /// - Returns Ok(R) where R is the result returned by the function `f`.
    ///
    /// ## Errors
    /// - Returns `Self::Error` if an error occurs while reading from the stream.
    ///
    async fn read_with<F, R>(&mut self, f: F) -> Result<R, Self::Error>
    where
        F: FnOnce(&mut [u8]) -> (usize, R);
}

/// Implement ReadWith for mutable references to types that implement ReadWith
impl<T: ?Sized + SocketReadWith> SocketReadWith for &mut T {
    #[inline]
    async fn read_with<F, R>(&mut self, f: F) -> Result<R, Self::Error>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        T::read_with(self, f).await
    }
}
/// Trait representing a write stream interface
pub trait SocketWriteWith: SocketErrorType {
    /// Write to the stream using the provided function
    ///
    /// The function `f` is called with a slice of available data from the stream.
    /// It should return a tuple containing the number of bytes written and a result value.
    ///
    /// ## Returns
    /// - Returns Ok(R) where R is the result returned by the function `f`.
    ///
    /// ## Errors
    /// - Returns `Self::Error` if an error occurs while writing to the stream.
    ///
    async fn write_with<F, R>(&mut self, f: F) -> Result<R, Self::Error>
    where
        F: FnOnce(&mut [u8]) -> (usize, R);
}

/// Implement WriteWith for mutable references to types that implement WriteWith
impl<T: ?Sized + SocketWriteWith> SocketWriteWith for &mut T {
    #[inline]
    async fn write_with<F, R>(&mut self, f: F) -> Result<R, Self::Error>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        T::write_with(self, f).await
    }
}

/// Socket close trait for TCP sockets, allowing for graceful shutdown of connections.
pub trait SocketClose {
    /// The error type that may be returned when closing the socket.
    type Error;

    /// Close the TCP socket gracefully, ensuring that all pending data is sent and acknowledged before closing the connection.
    /// This method should handle the TCP connection teardown process, including sending FIN packets and waiting for ACKs from the remote endpoint.
    /// ## Returns
    /// - Returns Ok(()) if the socket was closed successfully.
    /// - Returns `Self::Error` if an error occurs while closing the socket.
    async fn close(&mut self) -> Result<(), Self::Error>;
}

/// Implement SocketClose for mutable references to types that implement SocketClose
impl<T: ?Sized + SocketClose> SocketClose for &mut T {
    type Error = T::Error;

    #[inline]
    async fn close(&mut self) -> Result<(), Self::Error> {
        T::close(self).await
    }
}

/// A type representing a socket endpoint, which includes an IP address and a port number.
pub type SocketEndpoint = ::core::net::SocketAddr;

/// A trait representing a TCP socket, which includes methods for retrieving socket information,
/// connecting to remote endpoints, accepting incoming connections, and performing asynchronous
/// read/write operations.
/// This trait is designed to be implemented by various socket types, allowing for a consistent
/// interface for TCP socket operations across different platforms and implementations.
/// The `Socket` trait encompasses all socket-related functionality, while the `SocketExtended`
/// trait includes additional methods for custom buffer management during read/write operations.
/// Implementers of the `Socket` trait must also implement the `SocketInfo`, `SocketClose`,
/// `SocketRead`, `SocketReadReady`, `SocketWrite`, and `SocketWriteReady` traits, while
/// implementers of the `SocketExtended` trait must also implement the `SocketReadWith` and
/// `SocketWriteWith` traits.
#[derive(Clone, Copy, PartialEq, Eq)]
#[defmt_or_log::derive_format_or_debug]
pub enum State {
    /// The socket is closed and not connected to any remote endpoint.
    Closed,
    /// The socket is in the process of being opened and is waiting for a connection to be established.
    Listen,
    /// The socket is in the process of connecting to a remote endpoint and is waiting for a response.
    SynSent,
    /// The socket has received a connection request and is waiting for the connection to be established.
    SynReceived,
    /// The socket is connected to a remote endpoint and is ready for data transfer.
    Established,
    /// The socket is in the process of closing the connection and is waiting for all pending data to be sent and acknowledged.
    FinWait1,
    /// The socket is in the process of closing the connection and is waiting for all pending data to be sent and acknowledged.
    FinWait2,
    /// The socket has received a connection close request from the remote endpoint and is waiting for the connection to be closed.
    CloseWait,
    /// The socket is in the process of closing the connection and is waiting for all pending data to be sent and acknowledged.
    Closing,
    /// The socket has sent a connection close request and is waiting for an acknowledgment from the remote endpoint.
    LastAck,
    /// The socket is in the TIME-WAIT state, waiting for enough time to pass to ensure the remote endpoint received the acknowledgment of its connection close request.
    TimeWait,
}

/// A trait representing socket information, which includes the local and remote endpoints.
pub trait SocketInfo {
    /// Get the local endpoint of the socket.
    ///
    /// Returns `None` if the socket is not bound (listening) or not connected.
    fn local_endpoint(&self) -> Option<SocketEndpoint>;

    /// Get the remote endpoint of the socket.
    ///
    /// Returns `None` if the socket is not connected.
    fn remote_endpoint(&self) -> Option<SocketEndpoint>;

    /// Get the current state of the socket, which can be used to determine if the socket is ready
    /// for accepting new connections or if it is still in the process of closing a previous
    /// connection.
    fn state(&self) -> State;
}

/// Implement SocketInfo for immutable references to types that implement SocketInfo
impl<T: ?Sized + SocketInfo> SocketInfo for &T {
    #[inline]
    fn local_endpoint(&self) -> Option<SocketEndpoint> {
        T::local_endpoint(self)
    }

    #[inline]
    fn remote_endpoint(&self) -> Option<SocketEndpoint> {
        T::remote_endpoint(self)
    }

    #[inline]
    fn state(&self) -> State {
        T::state(self)
    }
}

/// A trait that provides a method for waiting until a socket is ready for reading.
pub trait SocketWaitReadReady: SocketErrorType {
    /// Wait until the socket is ready for reading, which means that there is data available to read
    /// from the socket.
    ///
    /// ## Returns
    /// - Returns Ok(()) if the socket is ready for reading.
    /// - Returns `Self::Error` if the socket is not readable anymore, which may occur if the connection
    /// has been closed or if an error occurs while waiting for the socket to become ready.
    async fn wait_read_ready(&mut self) -> Result<(), Self::Error>;
}

/// Implement SocketWaitReadReady for immutable references to types that implement SocketWaitReadReady
impl<T: ?Sized + SocketWaitReadReady> SocketWaitReadReady for &mut T {
    #[inline]
    async fn wait_read_ready(&mut self) -> Result<(), Self::Error> {
        T::wait_read_ready(self).await
    }
}

/// A trait that provides a method for waiting until a socket is ready for writing.
pub trait SocketWaitWriteReady: SocketErrorType {
    /// Wait until the socket is ready for writing, which means that the socket can accept data to be written
    /// without blocking.
    ///
    /// ## Returns
    /// - Returns Ok(()) if the socket is ready for writing.
    /// - Returns `Self::Error` if the socket is not writable anymore, which may occur if the connection
    /// has been closed or if an error occurs while waiting for the socket to become ready
    async fn wait_write_ready(&mut self) -> Result<(), Self::Error>;
}

/// Implement SocketWaitWriteReady for immutable references to types that implement SocketWaitWriteReady
impl<T: ?Sized + SocketWaitWriteReady> SocketWaitWriteReady for &mut T {
    #[inline]
    async fn wait_write_ready(&mut self) -> Result<(), Self::Error> {
        T::wait_write_ready(self).await
    }
}
/// A trait that encompasses all socket-related functionality, including information retrieval, graceful shutdown,
/// and asynchronous read/write operations with custom buffer management.
pub trait SocketStream:
    SocketRead + SocketReadReady + SocketWrite + SocketWriteReady + SocketWaitReadReady + SocketWaitWriteReady
{
}
impl<
    T: ?Sized + SocketRead + SocketReadReady + SocketWrite + SocketWriteReady + SocketWaitReadReady + SocketWaitWriteReady,
> SocketStream for T
{
}

/// A trait that encompasses all socket-related functionality, including information retrieval, graceful shutdown,
/// and asynchronous read/write operations with custom buffer management.
/// This trait is designed to be implemented by various socket types, allowing for a consistent interface for TCP
/// socket operations across different platforms and implementations. Implementers of the `Socket` trait must also
/// implement the `SocketInfo`, `SocketClose`, `SocketRead`, `SocketReadReady`, `SocketWrite`, `SocketWriteReady`,
/// `SocketReadWith`, and `SocketWriteWith` traits.
pub trait AbstractSocket: SocketStream + SocketInfo + SocketClose {}
impl<T: ?Sized + SocketStream + SocketInfo + SocketClose> AbstractSocket for T {}

/// A trait that encompasses all socket-related functionality, including information retrieval, graceful shutdown,
/// and asynchronous read/write operations with custom buffer management.
/// This trait is designed to be implemented by various socket types, allowing for a consistent interface for TCP
/// socket operations across different platforms and implementations. Implementers of the `Socket` trait must also
/// implement the `SocketInfo`, `SocketClose`, `SocketRead`, `SocketReadReady`, `SocketWrite`, `SocketWriteReady`,
/// `SocketReadWith`, and `SocketWriteWith` traits.
pub trait ExtendedSocket: AbstractSocket + SocketReadWith + SocketWriteWith {}
impl<T: ?Sized + AbstractSocket + SocketReadWith + SocketWriteWith> ExtendedSocket for T {}

/// A trait representing a socket listener, which provides methods for constructing socket instances and retrieving
/// socket endpoint information. This trait is designed to be implemented by various socket listener types, allowing
/// for a consistent interface for constructing socket instances across different platforms and implementations. The
/// `AbstractSocketListener` trait includes an associated type `Socket` that represents the type of socket produced
/// by the listener, and methods for accepting incoming connections and retrieving the socket endpoint.
/// Implementers of the `AbstractSocketListener` trait must provide an implementation for the `accept` method, which
/// constructs a new socket instance based on the listener's configuration, and the `endpoint` method, which returns
/// the socket endpoint that the listener is configured to listen on.
/// The `AbstractSocketListener` trait is designed to be flexible and extensible, allowing for different types of
/// socket listeners to be implemented while still adhering to a common interface for constructing socket instances
/// and retrieving endpoint information.
pub trait AbstractSocketListener {
    /// The associated type representing the socket produced by the listener.
    /// The produced socket has a lifetime parameter that is tied to the listener, ensuring that the socket cannot
    /// outlive the listener that created it.
    type Socket;

    /// Accept an incoming connection and construct a new socket instance based on the listener's configuration.
    /// This method should block until a new connection is accepted.
    ///
    /// ### Returns
    /// - Returns an instance of `Self::Socket` representing the accepted connection if successful.
    ///
    /// Note: this method should not panic on errors.
    async fn accept(&self) -> Self::Socket;

    /// Attempt to accept an incoming connection without blocking. This method should return immediately, indicating
    /// whether a new connection was accepted or if no connections are currently pending.
    /// ### Returns
    /// - Returns `Some(Self::Socket)` if a new connection was accepted successfully.
    /// - Returns `None` if no connections are currently pending or if an error occurs while attempting to accept a connection.
    async fn try_accept(&self) -> Option<Self::Socket>;

    /// Get the local endpoint that the listener is configured to listen on. This method should return the socket endpoint
    /// that the listener is bound to, which can be used by clients to connect to the listener.
    ///
    /// ### Returns
    /// - Returns a `SocketEndpoint` representing the local endpoint that the listener is configured to listen on.
    ///
    /// ### Panics
    /// This method may panic if the listener is not properly initialized or if there is an error retrieving the local
    /// endpoint information. Implementers of this trait should ensure that the listener is properly initialized and ready to
    /// provide the local endpoint information before calling this method.
    fn local_endpoint(&self) -> SocketEndpoint;
}

impl<T: ?Sized + AbstractSocketListener> AbstractSocketListener for &T {
    type Socket = T::Socket;

    #[inline]
    async fn accept(&self) -> Self::Socket {
        T::accept(self).await
    }

    #[inline]
    async fn try_accept(&self) -> Option<Self::Socket> {
        T::try_accept(self).await
    }

    #[inline]
    fn local_endpoint(&self) -> SocketEndpoint {
        T::local_endpoint(self)
    }
}

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
pub trait AbstarctSocketConnector {
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
    /// representing the established connection or an error if the connection attempt fails.
    async fn connect(&self, endpoint: SocketEndpoint) -> Result<Self::Socket, Self::Error>;
}

impl<T: ?Sized + AbstarctSocketConnector> AbstarctSocketConnector for &T {
    type Error = T::Error;
    type Socket = T::Socket;

    #[inline]
    async fn connect(&self, endpoint: SocketEndpoint) -> Result<Self::Socket, Self::Error> {
        T::connect(self, endpoint).await
    }
}
