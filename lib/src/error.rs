use crate::abstarct_socket::stream_search::StreamReadError;

/// Errors that can occur during HTTP operations
///
/// This enum represents all possible errors that can be returned by the HTTP client
/// during various stages of request processing, from URL parsing to connection
/// establishment and response handling.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[defmt_or_log::maybe_derive_format]
/// All possible errors returned by the HTTP client.
pub enum Error {
    /// TCP read/write error
    SocketError,
    /// Memory overflowed
    MemoryOverflow,
    /// No response was received from the server
    ServerError,
    /// The response/request could not be parsed due to invalid data
    MalformedRequest(&'static str),
}

impl<SocketReadErrorT> From<StreamReadError<SocketReadErrorT>> for Error
where
    Error: From<SocketReadErrorT>,
{
    fn from(err: StreamReadError<SocketReadErrorT>) -> Self {
        match err {
            StreamReadError::SocketReadError(e) => Error::from(e),
            StreamReadError::ReadBufferOverflow => Error::MemoryOverflow,
        }
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::SocketError => write!(f, "Socket error"),
            Error::MemoryOverflow => write!(f, "Memory overflowed"),
            Error::ServerError => write!(f, "No response received from server"),
            Error::MalformedRequest(msg) => write!(f, "Malformed request: {msg}"),
        }
    }
}

impl<ErrorT> From<embedded_io_async::ReadExactError<ErrorT>> for Error
where
    ErrorT: embedded_io_async::Error,
    Error: From<ErrorT>,
{
    fn from(err: embedded_io_async::ReadExactError<ErrorT>) -> Self {
        match err {
            embedded_io_async::ReadExactError::UnexpectedEof => Error::SocketError,
            embedded_io_async::ReadExactError::Other(e) => Error::from(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct DummySocketError;

    impl From<DummySocketError> for Error {
        fn from(_: DummySocketError) -> Self {
            Error::SocketError
        }
    }

    #[test]
    fn test_from_read_error() {
        let read_error = StreamReadError::SocketReadError(DummySocketError);
        let err: Error = read_error.into();
        assert!(matches!(err, Error::SocketError));

        let read_error = StreamReadError::<DummySocketError>::ReadBufferOverflow;
        let err: Error = read_error.into();
        assert!(matches!(err, Error::MemoryOverflow));
    }

    #[test]
    fn test_error_display() {
        let e = Error::SocketError;
        let mut str = heapless::String::<32>::new();
        core::fmt::write(&mut str, format_args!("{e}")).unwrap();
        assert_eq!(str, "Socket error");
        let e = Error::MemoryOverflow;
        let mut str = heapless::String::<32>::new();
        core::fmt::write(&mut str, format_args!("{e}")).unwrap();
        assert_eq!(str, "Memory overflowed");
        let e = Error::ServerError;
        let mut str = heapless::String::<32>::new();
        core::fmt::write(&mut str, format_args!("{e}")).unwrap();
        assert_eq!(str, "No response received from server");
        let e = Error::MalformedRequest("bad");
        let mut str = heapless::String::<32>::new();
        core::fmt::write(&mut str, format_args!("{e}")).unwrap();
        assert_eq!(str, "Malformed request: bad");
    }

    #[test]
    fn test_from_socket_error() {
        let tcp_err = DummySocketError;
        let err: Error = tcp_err.into();
        match err {
            Error::SocketError => {}
            _ => panic!("Expected SocketError variant"),
        }
    }
}
