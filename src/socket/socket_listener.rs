#![allow(async_fn_in_trait)]
use crate::socket::SocketEndpoint;

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
