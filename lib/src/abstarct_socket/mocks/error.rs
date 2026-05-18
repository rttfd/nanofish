/// Error returned by stream read functions.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum MockStreamError {
    /// The connection was reset.
    ///
    /// This can happen on receiving a RST packet, or on timeout.
    ConnectionReset,
}

/// Alias for write errors returned by stream write functions.
pub type MockReadError = MockStreamError;
/// Alias for write errors returned by stream write functions.
pub type MockWriteError = MockStreamError;

mod embedded_io_impls {
    use super::*;

    impl embedded_io_async::Error for MockStreamError {
        fn kind(&self) -> embedded_io_async::ErrorKind {
            match self {
                MockStreamError::ConnectionReset => embedded_io_async::ErrorKind::ConnectionReset,
            }
        }
    }
}
