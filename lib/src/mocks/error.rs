use crate::abstarct_socket::mocks::error::MockStreamError;
use crate::error::Error;

impl From<MockStreamError> for Error {
    fn from(err: MockStreamError) -> Self {
        match err {
            MockStreamError::ConnectionReset => Error::SocketError,
        }
    }
}
