/// This module contains the implementation of socket traits for the embassy_net tcp socket,
/// which provides asynchronous read/write operations and socket information retrieval.
pub mod socket;
/// This module contains the TcpSocketPool implementation for managing a pool of TCP sockets
/// using embassy-net, enabling concurrent acceptance of multiple incoming connections.
pub mod tcp_socket_pool;
