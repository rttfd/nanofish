//! HTTP server ports and adapters.

#[cfg(feature = "embassy")]
mod embassy;
mod io;

/// HTTP server timeout configuration.
#[derive(Debug, Clone, Copy)]
pub struct ServerTimeouts {
    /// Socket accept timeout in seconds.
    pub accept_timeout: u64,
    /// Socket read timeout in seconds.
    pub read_timeout: u64,
    /// Request handler timeout in seconds.
    pub handler_timeout: u64,
}

impl Default for ServerTimeouts {
    fn default() -> Self {
        Self {
            accept_timeout: 10,
            read_timeout: 30,
            handler_timeout: 60,
        }
    }
}

impl ServerTimeouts {
    /// Create new server timeouts with custom values.
    #[must_use]
    pub const fn new(accept_timeout: u64, read_timeout: u64, handler_timeout: u64) -> Self {
        Self {
            accept_timeout,
            read_timeout,
            handler_timeout,
        }
    }
}

#[cfg(feature = "embassy")]
pub use embassy::{DefaultEmbassyHttpServer, EmbassyHttpServer, SmallEmbassyHttpServer};
pub use io::{
    DefaultHttpIoServer, HttpIoServer, SmallHttpIoServer, handle_http_connection,
    handle_http_connection_with_sizes,
};
