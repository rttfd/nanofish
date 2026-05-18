use crate::abstarct_socket::stream_search::StreamReadError;

impl From<embassy_net::tcp::Error> for StreamReadError<embassy_net::tcp::Error> {
    fn from(err: embassy_net::tcp::Error) -> Self {
        StreamReadError::SocketReadError(err)
    }
}
