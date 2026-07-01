//! smoltcp adapters for nanofish's transport-generic IO APIs.
//!
//! This module provides an [`embedded_io_async`] stream wrapper around a
//! `smoltcp` TCP socket. The application still owns the `smoltcp` interface,
//! device polling, socket creation, connection/listen setup, and timeouts.

use core::{fmt, future::poll_fn, task::Poll};

use embedded_io_async::{Error, ErrorKind, ErrorType, Read, Write};
use smoltcp::socket::tcp::{RecvError, SendError, Socket};

/// Async IO adapter for a `smoltcp` TCP socket.
///
/// Use this with [`crate::HttpClient`], [`crate::HttpServer`], or
/// [`crate::handle_http_connection`] once the socket is connected or accepted.
/// The surrounding application must keep polling the `smoltcp` interface so the
/// socket can make progress and wake pending reads/writes.
pub struct SmolTcpStream<'socket, 'buffer> {
    socket: &'socket mut Socket<'buffer>,
}

impl<'socket, 'buffer> SmolTcpStream<'socket, 'buffer> {
    /// Wrap a `smoltcp` TCP socket as an `embedded-io-async` stream.
    #[must_use]
    pub const fn new(socket: &'socket mut Socket<'buffer>) -> Self {
        Self { socket }
    }

    /// Return a shared reference to the wrapped socket.
    #[must_use]
    pub const fn socket(&self) -> &Socket<'buffer> {
        self.socket
    }

    /// Return a mutable reference to the wrapped socket.
    #[must_use]
    pub const fn socket_mut(&mut self) -> &mut Socket<'buffer> {
        self.socket
    }

    /// Consume the adapter and return the wrapped socket reference.
    #[must_use]
    pub const fn into_inner(self) -> &'socket mut Socket<'buffer> {
        self.socket
    }
}

impl ErrorType for SmolTcpStream<'_, '_> {
    type Error = SmolTcpError;
}

impl Read for SmolTcpStream<'_, '_> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }

        poll_fn(|cx| match self.socket.recv_slice(buf) {
            Ok(0) if self.socket.may_recv() => {
                self.socket.register_recv_waker(cx.waker());
                Poll::Pending
            }
            Ok(n) => Poll::Ready(Ok(n)),
            Err(RecvError::Finished) => Poll::Ready(Ok(0)),
            Err(RecvError::InvalidState) => Poll::Ready(Err(SmolTcpError::RecvInvalidState)),
        })
        .await
    }
}

impl Write for SmolTcpStream<'_, '_> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }

        poll_fn(|cx| match self.socket.send_slice(buf) {
            Ok(0) if self.socket.may_send() => {
                self.socket.register_send_waker(cx.waker());
                Poll::Pending
            }
            Ok(n) => Poll::Ready(Ok(n)),
            Err(SendError::InvalidState) => Poll::Ready(Err(SmolTcpError::SendInvalidState)),
        })
        .await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        poll_fn(|cx| {
            if self.socket.send_queue() == 0 {
                Poll::Ready(Ok(()))
            } else if self.socket.may_send() {
                self.socket.register_send_waker(cx.waker());
                Poll::Pending
            } else {
                Poll::Ready(Err(SmolTcpError::SendInvalidState))
            }
        })
        .await
    }
}

/// Error returned by [`SmolTcpStream`].
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SmolTcpError {
    /// The socket receive half is not open for this operation.
    RecvInvalidState,
    /// The socket transmit half is not open for this operation.
    SendInvalidState,
}

impl Error for SmolTcpError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::RecvInvalidState | Self::SendInvalidState => ErrorKind::NotConnected,
        }
    }
}

impl core::error::Error for SmolTcpError {}

impl fmt::Display for SmolTcpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RecvInvalidState => f.write_str("smoltcp receive half is not open"),
            Self::SendInvalidState => f.write_str("smoltcp transmit half is not open"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_kind_is_not_connected() {
        assert_eq!(
            SmolTcpError::RecvInvalidState.kind(),
            ErrorKind::NotConnected
        );
        assert_eq!(
            SmolTcpError::SendInvalidState.kind(),
            ErrorKind::NotConnected
        );
    }
}
