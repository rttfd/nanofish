extern crate alloc;

use alloc::vec::Vec;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf, ReadHalf, WriteHalf};
use tokio::net::{TcpSocket, TcpStream};

use crate::socket::{
    SocketClose, SocketEndpoint, SocketErrorKind, SocketErrorType, SocketInfo, SocketRead, SocketReadReady,
    SocketReadWith, SocketWaitReadReady, SocketWaitWriteReady, SocketWrite, SocketWriteReady, SocketWriteWith,
};

/// Error type used by the Tokio adapters in this crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokioSocketError(pub embedded_io_async::ErrorKind);

const READ_BUFFER_SIZE: usize = 1024;
const WRITE_BUFFER_SIZE: usize = 1024;

/// A Tokio-backed wrapper around TCP sockets, listeners, and connected streams.
pub enum TokioSocketState {
    /// A configurable socket that has not connected yet.
    Socket(TcpSocket),
    /// A connected TCP stream.
    Stream(TcpStream),
}

/// A wrapper around Tokio TCP sockets and streams that implements the traits defined in the `socket` module.
pub struct TokioSocketWrapper {
    state: TokioSocketState,
}

impl TokioSocketWrapper {
    /// Creates a wrapper around a connected Tokio stream.
    pub(crate) fn new_stream(stream: TcpStream) -> Self {
        Self {
            state: TokioSocketState::Stream(stream),
        }
    }
}

impl SocketErrorType for TokioSocketWrapper {
    type Error = TokioSocketError;
}

impl SocketReadWith for TokioSocketWrapper {
    async fn read_with<F, R>(&mut self, f: F) -> Result<R, Self::Error>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        match &mut self.state {
            TokioSocketState::Stream(stream) => {
                stream.readable().await?;
                let mut buf = [0u8; READ_BUFFER_SIZE];
                let peeked = stream.peek(&mut buf).await?;
                let (consumed, result) = f(&mut buf[..peeked]);
                assert!(consumed <= peeked, "Read more bytes than available in buffer");
                if consumed > 0 {
                    stream.read_exact(&mut buf[..consumed]).await?;
                }
                Ok(result)
            }
            TokioSocketState::Socket(_) => {
                Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Tokio socket is not connected").into())
            }
        }
    }
}

impl<'socket> SocketReadWith for TokioSocketReadHalfWrapper<'socket> {
    async fn read_with<F, R>(&mut self, f: F) -> Result<R, Self::Error>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        if self.pending_mut().is_empty() {
            let mut buf = [0u8; READ_BUFFER_SIZE];
            let read = self.inner_mut().read(&mut buf).await?;
            self.pending_mut().extend_from_slice(&buf[..read]);
        }

        Ok(apply_read_with(self.pending_mut(), f))
    }
}

impl SocketReadWith for TokioSocketOwnedReadHalfWrapper {
    async fn read_with<F, R>(&mut self, f: F) -> Result<R, Self::Error>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        if self.pending_mut().is_empty() {
            let mut buf = [0u8; READ_BUFFER_SIZE];
            let read = self.inner_mut().read(&mut buf).await?;
            self.pending_mut().extend_from_slice(&buf[..read]);
        }

        Ok(apply_read_with(self.pending_mut(), f))
    }
}

impl SocketWriteWith for TokioSocketWrapper {
    async fn write_with<F, R>(&mut self, f: F) -> Result<R, Self::Error>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        match &mut self.state {
            TokioSocketState::Stream(stream) => {
                stream.writable().await?;
                let mut buf = [0u8; WRITE_BUFFER_SIZE];
                let (written, result) = f(&mut buf);
                assert!(written <= buf.len(), "Wrote more bytes than available in buffer");
                stream.write_all(&buf[..written]).await?;
                Ok(result)
            }
            TokioSocketState::Socket(_) => {
                Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Tokio socket is not connected").into())
            }
        }
    }
}

impl<'socket> SocketWriteWith for TokioSocketWriteHalfWrapper<'socket> {
    async fn write_with<F, R>(&mut self, f: F) -> Result<R, Self::Error>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        let mut buf = [0u8; WRITE_BUFFER_SIZE];
        let (written, result) = f(&mut buf);
        assert!(written <= buf.len(), "Wrote more bytes than available in buffer");
        self.inner_mut().write_all(&buf[..written]).await?;
        Ok(result)
    }
}

impl SocketWriteWith for TokioSocketOwnedWriteHalfWrapper {
    async fn write_with<F, R>(&mut self, f: F) -> Result<R, Self::Error>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        let mut buf = [0u8; WRITE_BUFFER_SIZE];
        let (written, result) = f(&mut buf);
        assert!(written <= buf.len(), "Wrote more bytes than available in buffer");
        self.inner_mut().write_all(&buf[..written]).await?;
        Ok(result)
    }
}

impl From<std::io::Error> for TokioSocketError {
    fn from(value: std::io::Error) -> Self {
        Self(match value.kind() {
            std::io::ErrorKind::NotFound => embedded_io_async::ErrorKind::NotFound,
            std::io::ErrorKind::PermissionDenied => embedded_io_async::ErrorKind::PermissionDenied,
            std::io::ErrorKind::ConnectionRefused => embedded_io_async::ErrorKind::ConnectionRefused,
            std::io::ErrorKind::ConnectionReset => embedded_io_async::ErrorKind::ConnectionReset,
            std::io::ErrorKind::ConnectionAborted => embedded_io_async::ErrorKind::ConnectionAborted,
            std::io::ErrorKind::NotConnected => embedded_io_async::ErrorKind::NotConnected,
            std::io::ErrorKind::AddrInUse => embedded_io_async::ErrorKind::AddrInUse,
            std::io::ErrorKind::AddrNotAvailable => embedded_io_async::ErrorKind::AddrNotAvailable,
            std::io::ErrorKind::BrokenPipe => embedded_io_async::ErrorKind::BrokenPipe,
            std::io::ErrorKind::AlreadyExists => embedded_io_async::ErrorKind::AlreadyExists,
            std::io::ErrorKind::InvalidInput => embedded_io_async::ErrorKind::InvalidInput,
            std::io::ErrorKind::InvalidData => embedded_io_async::ErrorKind::InvalidData,
            std::io::ErrorKind::TimedOut => embedded_io_async::ErrorKind::TimedOut,
            std::io::ErrorKind::Interrupted => embedded_io_async::ErrorKind::Interrupted,
            std::io::ErrorKind::Unsupported => embedded_io_async::ErrorKind::Unsupported,
            std::io::ErrorKind::OutOfMemory => embedded_io_async::ErrorKind::OutOfMemory,
            std::io::ErrorKind::WriteZero => embedded_io_async::ErrorKind::WriteZero,
            _ => embedded_io_async::ErrorKind::Other,
        })
    }
}

impl embedded_io_async::Error for TokioSocketError {
    fn kind(&self) -> embedded_io_async::ErrorKind {
        self.0
    }
}

impl SocketRead for TokioSocketWrapper {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        match &mut self.state {
            TokioSocketState::Stream(stream) => stream.read(buf).await.map_err(Into::into),
            TokioSocketState::Socket(_) => Err(invalid_input_error("Tokio socket is not connected").into()),
        }
    }
}

impl SocketWrite for TokioSocketWrapper {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        match &mut self.state {
            TokioSocketState::Stream(stream) => stream.write(buf).await.map_err(Into::into),
            TokioSocketState::Socket(_) => Err(invalid_input_error("Tokio socket is not connected").into()),
        }
    }
}

impl SocketReadReady for TokioSocketWrapper {
    fn read_ready(&mut self) -> Result<bool, Self::Error> {
        match &mut self.state {
            TokioSocketState::Stream(_) => Ok(true),
            TokioSocketState::Socket(_) => Ok(false),
        }
    }
}

impl SocketWriteReady for TokioSocketWrapper {
    fn write_ready(&mut self) -> Result<bool, Self::Error> {
        match &mut self.state {
            TokioSocketState::Stream(stream) => match stream.try_write(&[]) {
                Ok(_) => Ok(true),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(false),
                Err(error) => Err(error.into()),
            },
            TokioSocketState::Socket(_) => Ok(false),
        }
    }
}

impl SocketInfo for TokioSocketWrapper {
    fn local_endpoint(&self) -> Option<SocketEndpoint> {
        match &self.state {
            TokioSocketState::Socket(socket) => {
                let local_addr = socket.local_addr().ok()?;
                let port = local_addr.port();
                Some(SocketEndpoint::new(local_addr.ip(), port))
            }
            TokioSocketState::Stream(stream) => {
                let local_addr = stream.local_addr().ok()?;
                let port = local_addr.port();
                Some(SocketEndpoint::new(local_addr.ip(), port))
            }
        }
    }

    fn remote_endpoint(&self) -> Option<SocketEndpoint> {
        match &self.state {
            TokioSocketState::Stream(stream) => {
                let remote_addr = stream.peer_addr().ok()?;
                let port = remote_addr.port();
                Some(SocketEndpoint::new(remote_addr.ip(), port))
            }
            TokioSocketState::Socket(_) => None,
        }
    }

    fn state(&self) -> crate::socket::State {
        match &self.state {
            TokioSocketState::Stream(_) => crate::socket::State::Established,
            TokioSocketState::Socket(_) => crate::socket::State::Closed,
        }
    }
}

impl SocketClose for TokioSocketWrapper {
    type Error = TokioSocketError;

    async fn close(&mut self) -> Result<(), Self::Error> {
        match &mut self.state {
            TokioSocketState::Stream(stream) => stream.shutdown().await.map_err(Into::into),
            TokioSocketState::Socket(_) => Ok(()),
        }
    }
}

fn invalid_input_error(message: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message)
}

fn drain_pending_bytes(pending: &mut Vec<u8>, buf: &mut [u8]) -> usize {
    let copied = core::cmp::min(pending.len(), buf.len());
    buf[..copied].copy_from_slice(&pending[..copied]);
    pending.drain(..copied);
    copied
}

fn apply_read_with<F, R>(pending: &mut Vec<u8>, f: F) -> R
where
    F: FnOnce(&mut [u8]) -> (usize, R),
{
    let (consumed, result) = f(pending.as_mut_slice());
    assert!(consumed <= pending.len(), "Read more bytes than available in buffer");
    pending.drain(..consumed);
    result
}

/// A `ReadWith`-compatible wrapper around a borrowed Tokio read half.
pub struct TokioSocketReadHalfWrapper<'socket> {
    inner: ReadHalf<'socket>,
    pending: Vec<u8>,
}

impl<'socket> TokioSocketReadHalfWrapper<'socket> {
    /// Creates a wrapper around a borrowed Tokio read half.
    pub fn new(read_half: ReadHalf<'socket>) -> Self {
        Self {
            inner: read_half,
            pending: Vec::new(),
        }
    }

    pub(crate) fn inner_mut(&mut self) -> &mut ReadHalf<'socket> {
        &mut self.inner
    }

    pub(crate) fn pending_mut(&mut self) -> &mut Vec<u8> {
        &mut self.pending
    }
}

impl SocketErrorType for TokioSocketReadHalfWrapper<'_> {
    type Error = TokioSocketError;
}

impl SocketRead for TokioSocketReadHalfWrapper<'_> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if !self.pending.is_empty() {
            return Ok(drain_pending_bytes(&mut self.pending, buf));
        }
        self.inner.read(buf).await.map_err(Into::into)
    }
}

impl SocketReadReady for TokioSocketReadHalfWrapper<'_> {
    fn read_ready(&mut self) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

/// A `ReadWith`-compatible wrapper around an owned Tokio read half.
pub struct TokioSocketOwnedReadHalfWrapper {
    inner: OwnedReadHalf,
    pending: Vec<u8>,
}

impl TokioSocketOwnedReadHalfWrapper {
    /// Creates a wrapper around an owned Tokio read half.
    pub fn new(read_half: OwnedReadHalf) -> Self {
        Self {
            inner: read_half,
            pending: Vec::new(),
        }
    }

    pub(crate) fn inner_mut(&mut self) -> &mut OwnedReadHalf {
        &mut self.inner
    }

    pub(crate) fn pending_mut(&mut self) -> &mut Vec<u8> {
        &mut self.pending
    }
}

impl SocketErrorType for TokioSocketOwnedReadHalfWrapper {
    type Error = TokioSocketError;
}

impl SocketRead for TokioSocketOwnedReadHalfWrapper {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if !self.pending.is_empty() {
            return Ok(drain_pending_bytes(&mut self.pending, buf));
        }
        self.inner.read(buf).await.map_err(Into::into)
    }
}

impl SocketReadReady for TokioSocketOwnedReadHalfWrapper {
    fn read_ready(&mut self) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

/// A `WriteWith`-compatible wrapper around a borrowed Tokio write half.
pub struct TokioSocketWriteHalfWrapper<'socket>(WriteHalf<'socket>);

impl<'socket> TokioSocketWriteHalfWrapper<'socket> {
    /// Creates a wrapper around a borrowed Tokio write half.
    pub const fn new(write_half: WriteHalf<'socket>) -> Self {
        Self(write_half)
    }

    pub(crate) fn inner_mut(&mut self) -> &mut WriteHalf<'socket> {
        &mut self.0
    }
}

impl SocketErrorType for TokioSocketWriteHalfWrapper<'_> {
    type Error = TokioSocketError;
}

impl SocketWrite for TokioSocketWriteHalfWrapper<'_> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.0.write(buf).await.map_err(Into::into)
    }
}

impl SocketWriteReady for TokioSocketWriteHalfWrapper<'_> {
    fn write_ready(&mut self) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

/// A `WriteWith`-compatible wrapper around an owned Tokio write half.
pub struct TokioSocketOwnedWriteHalfWrapper(OwnedWriteHalf);

impl TokioSocketOwnedWriteHalfWrapper {
    /// Creates a wrapper around an owned Tokio write half.
    pub const fn new(write_half: OwnedWriteHalf) -> Self {
        Self(write_half)
    }

    pub(crate) fn inner_mut(&mut self) -> &mut OwnedWriteHalf {
        &mut self.0
    }
}

impl SocketErrorType for TokioSocketOwnedWriteHalfWrapper {
    type Error = TokioSocketError;
}

impl SocketWrite for TokioSocketOwnedWriteHalfWrapper {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.0.write(buf).await.map_err(Into::into)
    }
}

impl SocketWriteReady for TokioSocketOwnedWriteHalfWrapper {
    fn write_ready(&mut self) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

// TODO: Implement tests for this trait implementation.
impl SocketWaitReadReady for TokioSocketWrapper {
    async fn wait_read_ready(&mut self) -> Result<(), Self::Error> {
        match &self.state {
            TokioSocketState::Stream(stream) => stream
                .readable()
                .await
                .map_err(|_| TokioSocketError(SocketErrorKind::ConnectionReset)),
            TokioSocketState::Socket(_) => {
                panic!(
                    "Tokio sockets and listeners are never ready for reading, so wait_read_ready should not be called on them"
                );
            }
        }
    }
}

// TODO: Implement tests for this trait implementation.
impl SocketWaitReadReady for TokioSocketReadHalfWrapper<'_> {
    async fn wait_read_ready(&mut self) -> Result<(), Self::Error> {
        self.inner
            .readable()
            .await
            .map_err(|_| TokioSocketError(SocketErrorKind::ConnectionReset))
    }
}

// TODO: Implement tests for this trait implementation.
impl SocketWaitReadReady for TokioSocketOwnedReadHalfWrapper {
    async fn wait_read_ready(&mut self) -> Result<(), Self::Error> {
        self.inner
            .readable()
            .await
            .map_err(|_| TokioSocketError(SocketErrorKind::ConnectionReset))
    }
}

// TODO: Implement tests for this trait implementation.
impl SocketWaitWriteReady for TokioSocketWrapper {
    async fn wait_write_ready(&mut self) -> Result<(), Self::Error> {
        match &self.state {
            TokioSocketState::Stream(stream) => stream
                .writable()
                .await
                .map_err(|_| TokioSocketError(SocketErrorKind::ConnectionReset)),
            TokioSocketState::Socket(_) => {
                panic!(
                    "Tokio sockets and listeners are never ready for writing, so wait_write_ready should not be called on them"
                );
            }
        }
    }
}

// TODO: Implement tests for this trait implementation.
impl SocketWaitWriteReady for TokioSocketWriteHalfWrapper<'_> {
    async fn wait_write_ready(&mut self) -> Result<(), Self::Error> {
        self.0
            .writable()
            .await
            .map_err(|_| TokioSocketError(SocketErrorKind::ConnectionReset))
    }
}

// TODO: Implement tests for this trait implementation.
impl SocketWaitWriteReady for TokioSocketOwnedWriteHalfWrapper {
    async fn wait_write_ready(&mut self) -> Result<(), Self::Error> {
        self.0
            .writable()
            .await
            .map_err(|_| TokioSocketError(SocketErrorKind::ConnectionReset))
    }
}
