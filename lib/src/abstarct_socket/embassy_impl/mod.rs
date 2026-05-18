/// This module contains the implementation of socket traits for the embassy_net tcp socket,
/// which provides asynchronous read/write operations and socket information retrieval.
pub mod socket;
/// This module contains the implementation of From trait for converting embassy_net tcp errors into StreamReadError,
/// which is used for error handling in stream reading operations.
pub mod stream_read_error;
/// This module contains the TcpSocketPool implementation for managing a pool of TCP sockets
/// using embassy-net, enabling concurrent acceptance of multiple incoming connections.
pub mod tcp_socket_pool;
