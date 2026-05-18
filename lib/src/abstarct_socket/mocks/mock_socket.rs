pub use crate::abstarct_socket::mocks::error::MockStreamError;
use crate::abstarct_socket::socket::{SocketClose, SocketEndpoint, SocketInfo, SocketReadWith, SocketWriteWith, State};
use embedded_io_async::{ErrorType, Read, ReadReady, Write, WriteReady};
extern crate alloc;
extern crate std;
use alloc::boxed::Box;
use defmt_or_log as log;
use std::pin::Pin;

/// Mock implementations for testing purposes.
pub type ResultFuture<T, E> = Pin<Box<dyn core::future::Future<Output = Result<T, E>>>>;
type ReadCallback = dyn for<'a> FnMut(&'a mut [u8]) -> ResultFuture<usize, MockStreamError>;
type WriteCallback = dyn for<'a> FnMut(&'a [u8]) -> ResultFuture<usize, MockStreamError>;
type ReadyCallback = dyn FnMut() -> Result<bool, MockStreamError>;
type CloseCallback = dyn FnMut() -> ResultFuture<(), MockStreamError>;
type EndpointCallback = dyn Fn() -> Option<SocketEndpoint>;
type StateCallback = dyn Fn() -> State;

/// A highly customizable mock socket implementation for testing purposes.
/// This struct allows users to set custom callbacks for read, write, readiness,
/// and close operations, enabling flexible simulation of various socket
/// behaviors in tests.
pub struct MockSocket {
    on_read: Option<Box<ReadCallback>>,
    on_write: Option<Box<WriteCallback>>,
    on_read_ready: Option<Box<ReadyCallback>>,
    on_write_ready: Option<Box<ReadyCallback>>,
    on_close: Option<Box<CloseCallback>>,
    on_local_endpoint: Option<Box<EndpointCallback>>,
    on_remote_endpoint: Option<Box<EndpointCallback>>,
    on_state: Option<Box<StateCallback>>,
}

/// Error returned by accept functions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MockAcceptError {
    /// The socket is already connected or listening.
    InvalidState,
    /// Invalid listen port
    InvalidPort,
    /// The remote host rejected the connection with a RST packet.
    ConnectionReset,
}

impl MockSocket {
    /// Create a new instance of MockSocket with no callbacks set. Users can set the desired
    /// callbacks using the provided setter methods.
    pub fn new() -> Self {
        Self {
            on_read: None,
            on_write: None,
            on_read_ready: None,
            on_write_ready: None,
            on_close: None,
            on_local_endpoint: None,
            on_remote_endpoint: None,
            on_state: None,
        }
    }

    /// Set the callback for the read (receive) operation. The callback should take a mutable byte slice
    /// and return the number of bytes read or an error.
    pub fn set_on_read<F>(&mut self, callback: F)
    where
        F: 'static + FnMut(&mut [u8]) -> ResultFuture<usize, MockStreamError>,
    {
        self.on_read = Some(Box::new(callback));
    }

    /// Set the callback for the write (send) operation. The callback should take a byte slice
    /// and return the number of bytes written or an error.
    pub fn set_on_write<F>(&mut self, callback: F)
    where
        F: 'static + FnMut(&[u8]) -> ResultFuture<usize, MockStreamError>,
    {
        self.on_write = Some(Box::new(callback));
    }

    /// Set the callback for the read readiness check. The callback should return true if the socket is ready to read,
    /// or false if it is not, along with any potential errors.
    pub fn set_on_read_ready<F>(&mut self, callback: F)
    where
        F: 'static + FnMut() -> Result<bool, MockStreamError>,
    {
        self.on_read_ready = Some(Box::new(callback));
    }

    /// Set the callback for the write readiness check. The callback should return true if the socket is ready to write,
    /// or false if it is not, along with any potential errors.
    pub fn set_on_write_ready<F>(&mut self, callback: F)
    where
        F: 'static + FnMut() -> Result<bool, MockStreamError>,
    {
        self.on_write_ready = Some(Box::new(callback));
    }

    /// Set the callback for the close operation. The callback should return Ok(()) if the socket was closed successfully,
    /// or an error if an error occurs while closing the socket.
    pub fn set_on_close<F>(&mut self, callback: F)
    where
        F: 'static + FnMut() -> ResultFuture<(), MockStreamError>,
    {
        self.on_close = Some(Box::new(callback));
    }

    /// Set the callback that returns the local endpoint of the mock socket.
    pub fn set_on_local_endpoint<F>(&mut self, callback: F)
    where
        F: 'static + Fn() -> Option<SocketEndpoint>,
    {
        self.on_local_endpoint = Some(Box::new(callback));
    }

    /// Set the callback that returns the remote endpoint of the mock socket.
    pub fn set_on_remote_endpoint<F>(&mut self, callback: F)
    where
        F: 'static + Fn() -> Option<SocketEndpoint>,
    {
        self.on_remote_endpoint = Some(Box::new(callback));
    }

    /// Set the callback that returns the state of the mock socket.
    pub fn set_on_state<F>(&mut self, callback: F)
    where
        F: 'static + Fn() -> State,
    {
        self.on_state = Some(Box::new(callback));
    }
}

impl Default for MockSocket {
    fn default() -> Self {
        Self::new()
    }
}

/***************************************************************************************************/
// Common callbacks for testing purposes
/***************************************************************************************************/

/// Callback helper that reports the socket as always ready to read.
pub fn on_read_ready_always_ready() -> Result<bool, MockStreamError> {
    Ok(true)
}
/// Callback helper that reports the socket as not ready to read.
pub fn on_read_ready_always_not_ready() -> Result<bool, MockStreamError> {
    Ok(false)
}

/// Callback helper that returns a read-readiness error.
pub fn on_read_ready_always_error() -> Result<bool, MockStreamError> {
    Err(MockStreamError::ConnectionReset)
}

/// Callback helper that reports the socket as always ready to write.
pub fn on_write_ready_always_ready() -> Result<bool, MockStreamError> {
    Ok(true)
}

/// Callback helper that reports the socket as not ready to write.
pub fn on_write_ready_always_not_ready() -> Result<bool, MockStreamError> {
    Ok(false)
}

/// Callback helper that returns a write-readiness error.
pub fn on_write_ready_always_error() -> Result<bool, MockStreamError> {
    Err(MockStreamError::ConnectionReset)
}

/// Callback helper that simulates a successful close operation.
pub fn on_close_always_succeed() -> Result<(), MockStreamError> {
    Ok(())
}
/// Callback helper that simulates a failed close operation.
pub fn on_close_always_fail() -> Result<(), MockStreamError> {
    Err(MockStreamError::ConnectionReset)
}

/// Callback helper that simulates an unset local endpoint.
pub fn on_local_endpoint_always_none() -> Option<SocketEndpoint> {
    None
}

/// Callback helper that simulates an unset remote endpoint.
pub fn on_remote_endpoint_always_none() -> Option<SocketEndpoint> {
    None
}

/// Construct a callback helper that simulates a socket state.
pub fn on_state_always(state: State) -> impl Fn() -> State {
    move || state
}

impl ErrorType for MockSocket {
    type Error = MockStreamError;
}

impl Read for MockSocket {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, MockStreamError> {
        if let Some(read_callback) = &mut self.on_read {
            let size = read_callback(buf).await?;
            Ok(size)
        } else {
            log::panic!("Read callback not set for MockSocket");
        }
    }
}

impl Write for MockSocket {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, MockStreamError> {
        if let Some(write_callback) = &mut self.on_write {
            let size = write_callback(buf).await?;
            Ok(size)
        } else {
            log::panic!("Write callback not set for MockSocket");
        }
    }
}

impl WriteReady for MockSocket {
    fn write_ready(&mut self) -> Result<bool, MockStreamError> {
        if let Some(on_write_ready) = self.on_write_ready.as_mut() {
            return on_write_ready();
        }
        log::panic!("Write ready callback not set for MockSocket");
    }
}

impl ReadReady for MockSocket {
    fn read_ready(&mut self) -> Result<bool, MockStreamError> {
        if let Some(on_read_ready) = self.on_read_ready.as_mut() {
            return on_read_ready();
        }
        log::panic!("Read ready callback not set for MockSocket");
    }
}

impl SocketReadWith for MockSocket {
    async fn read_with<F, R>(&mut self, f: F) -> Result<R, Self::Error>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        if let Some(read_callback) = &mut self.on_read {
            let mut buf = vec![0; 1024];
            let size = read_callback(buf.as_mut_slice()).await?;
            let (written_size, result) = f(&mut buf[..size]);
            assert!(written_size <= size, "Read more bytes than available in buffer");
            Ok(result)
        } else {
            log::panic!("Read callback not set for MockSocket");
        }
    }
}

impl SocketWriteWith for MockSocket {
    #[inline]
    async fn write_with<F, R>(&mut self, f: F) -> Result<R, Self::Error>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        if let Some(send_callback) = &mut self.on_write {
            let mut buf = vec![0; 1024];

            let (written_size, result) = f(buf.as_mut_slice());
            assert!(written_size <= buf.len(), "Wrote more bytes than available in buffer");
            send_callback(&buf[..written_size]).await?;

            Ok(result)
        } else {
            log::panic!("Write callback not set for MockSocket");
        }
    }
}

impl SocketClose for MockSocket {
    type Error = MockStreamError;

    async fn close(&mut self) -> Result<(), Self::Error> {
        if let Some(on_close) = &mut self.on_close {
            on_close().await
        } else {
            log::panic!("Close callback not set for MockSocket");
        }
    }
}

impl SocketInfo for MockSocket {
    fn local_endpoint(&self) -> Option<SocketEndpoint> {
        if let Some(on_local_endpoint) = &self.on_local_endpoint {
            on_local_endpoint()
        } else {
            log::panic!("Local endpoint callback not set for MockSocket");
        }
    }

    fn remote_endpoint(&self) -> Option<SocketEndpoint> {
        if let Some(on_remote_endpoint) = &self.on_remote_endpoint {
            on_remote_endpoint()
        } else {
            log::panic!("Remote endpoint callback not set for MockSocket");
        }
    }

    fn state(&self) -> State {
        if let Some(on_state) = &self.on_state {
            on_state()
        } else {
            log::panic!("State callback not set for MockSocket");
        }
    }
}

#[cfg(test)]
mod tests {
    use core::future::ready;

    use super::*;

    #[tokio::test]
    #[should_panic(expected = "Write callback not set for MockSocket")]
    async fn test_write_should_panic_if_callback_not_set() {
        let mut mock_socket = MockSocket::new();
        let write_data = b"Hello, World!";
        let _ = mock_socket.write(write_data).await;
    }

    #[tokio::test]
    #[should_panic(expected = "Read callback not set for MockSocket")]
    async fn test_read_should_panic_if_callback_not_set() {
        let mut mock_socket = MockSocket::new();
        let mut read_buf = [0u8; 20];
        let _ = mock_socket.read(&mut read_buf).await;
    }

    #[test]
    #[should_panic(expected = "Read ready callback not set for MockSocket")]
    fn test_read_ready_should_panic_if_callback_not_set() {
        let mut mock_socket = MockSocket::new();
        let _ = mock_socket.read_ready();
    }

    #[test]
    #[should_panic(expected = "Write ready callback not set for MockSocket")]
    fn test_write_ready_should_panic_if_callback_not_set() {
        let mut mock_socket = MockSocket::new();
        let _ = mock_socket.write_ready();
    }

    #[tokio::test]
    #[should_panic(expected = "Write callback not set for MockSocket")]
    async fn test_write_with_should_panic_if_callback_not_set() {
        let mut mock_socket = MockSocket::new();
        let _ = mock_socket.write_with(|_| (0, ())).await;
    }

    #[tokio::test]
    #[should_panic(expected = "Close callback not set for MockSocket")]
    async fn test_close_should_panic_if_callback_not_set() {
        let mut mock_socket = MockSocket::new();
        let _ = mock_socket.close().await;
    }

    #[test]
    #[should_panic(expected = "Local endpoint callback not set for MockSocket")]
    fn test_local_endpoint_should_panic_if_callback_not_set() {
        let mock_socket = MockSocket::new();
        let _ = mock_socket.local_endpoint();
    }

    #[test]
    #[should_panic(expected = "Remote endpoint callback not set for MockSocket")]
    fn test_remote_endpoint_should_panic_if_callback_not_set() {
        let mock_socket = MockSocket::new();
        let _ = mock_socket.remote_endpoint();
    }

    #[tokio::test]
    async fn test_mock_socket_read_write() {
        let mut mock_socket = MockSocket::new();

        mock_socket.set_on_write(|data| {
            assert_eq!(data, b"Hello, World!");
            Box::pin(ready(Ok(data.len())))
        });

        mock_socket.set_on_read(|buf| {
            let response = b"Response Data";
            let len = response.len().min(buf.len());
            buf[..len].copy_from_slice(&response[..len]);
            Box::pin(ready(Ok(len)))
        });

        let write_data = b"Hello, World!";
        let bytes_written = mock_socket.write(write_data).await.unwrap();
        assert_eq!(bytes_written, write_data.len());

        let mut read_buf = [0u8; 20];
        let bytes_read = mock_socket.read(&mut read_buf).await.unwrap();
        assert_eq!(&read_buf[..bytes_read], b"Response Data");
    }

    #[tokio::test]
    async fn test_mock_socket_partial_read() {
        let mut mock_socket = MockSocket::new();

        mock_socket.set_on_read(|buf| {
            let response = b"This is a very long response";
            let len = response.len().min(buf.len());
            buf[..len].copy_from_slice(&response[..len]);
            Box::pin(ready(Ok(len)))
        });

        let mut small_buf = [0u8; 10];
        let bytes_read = mock_socket.read(&mut small_buf).await.unwrap();
        assert_eq!(bytes_read, 10);
        assert_eq!(&small_buf, b"This is a ");
    }

    #[tokio::test]
    async fn test_mock_socket_error_in_callback() {
        let mut mock_socket = MockSocket::new();

        mock_socket.set_on_write(|_| Box::pin(async move { Err(MockStreamError::ConnectionReset) }));
        mock_socket.set_on_read(|_| Box::pin(async move { Err(MockStreamError::ConnectionReset) }));

        let write_data = b"test";
        let write_result = mock_socket.write(write_data).await;
        assert!(write_result.is_err());

        let mut read_buf = [0u8; 10];
        let read_result = mock_socket.read(&mut read_buf).await;
        assert!(read_result.is_err());
    }

    #[tokio::test]
    async fn test_mock_socket_zero_length_operations() {
        let mut mock_socket = MockSocket::new();

        mock_socket.set_on_write(|data| Box::pin(ready(Ok(data.len()))));
        mock_socket.set_on_read(|_| Box::pin(ready(Ok(0))));

        let empty_data = b"";
        let bytes_written = mock_socket.write(empty_data).await.unwrap();
        assert_eq!(bytes_written, 0);

        let mut read_buf = [0u8; 10];
        let bytes_read = mock_socket.read(&mut read_buf).await.unwrap();
        assert_eq!(bytes_read, 0);
    }

    #[tokio::test]
    async fn test_mock_socket_read_ready() {
        let mut mock_socket = MockSocket::new();

        mock_socket.set_on_read_ready(|| Ok(false));

        let mut is_ready = mock_socket.read_ready().unwrap();
        assert_eq!(is_ready, false);

        mock_socket.set_on_read_ready(|| Ok(true));

        is_ready = mock_socket.read_ready().unwrap();
        assert_eq!(is_ready, true);
    }

    #[tokio::test]
    async fn test_mock_socket_write_ready() {
        let mut mock_socket = MockSocket::new();

        mock_socket.set_on_write_ready(|| Ok(false));

        let mut is_ready = mock_socket.write_ready().unwrap();
        assert_eq!(is_ready, false);

        mock_socket.set_on_write_ready(|| Ok(true));

        is_ready = mock_socket.write_ready().unwrap();
        assert_eq!(is_ready, true);
    }

    #[test]
    fn test_mock_socket_local_remote_endpoint() {
        let local = SocketEndpoint::new(core::net::IpAddr::V4(core::net::Ipv4Addr::LOCALHOST), 8080);
        let remote = SocketEndpoint::new(core::net::IpAddr::V4(core::net::Ipv4Addr::new(10, 0, 0, 2)), 9090);

        let mut mock_socket = MockSocket::new();
        mock_socket.set_on_local_endpoint({
            let local = local.clone();
            move || Some(local.clone())
        });
        mock_socket.set_on_remote_endpoint({
            let remote = remote.clone();
            move || Some(remote.clone())
        });

        assert_eq!(mock_socket.local_endpoint(), Some(local));
        assert_eq!(mock_socket.remote_endpoint(), Some(remote));
    }

    #[test]
    fn test_mock_socket_state() {
        let mut mock_socket = MockSocket::new();
        mock_socket.set_on_state(on_state_always(State::Established));

        assert!(
            mock_socket.state() == State::Established,
            "Expected socket state to be Established"
        );
    }

    #[tokio::test]
    async fn test_mock_socket_close() {
        let mut mock_socket = MockSocket::new();
        mock_socket.set_on_close(|| Box::pin(async move { Ok(()) }));
        let close_result = mock_socket.close().await;
        assert!(close_result.is_ok());
    }
}
