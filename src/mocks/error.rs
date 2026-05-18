use crate::error::Error;
use crate::socket::mocks::error::MockStreamError;

impl From<MockStreamError> for Error {
    fn from(err: MockStreamError) -> Self {
        match err {
            MockStreamError::ConnectionReset => Error::SocketError,
        }
    }
}
