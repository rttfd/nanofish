//! HTTP server ports and adapters.

#[cfg(feature = "embassy")]
mod embassy;
mod io;

#[cfg(feature = "embassy")]
pub use embassy::{DefaultHttpServer, HttpServer, ServerTimeouts, SmallHttpServer};
pub use io::{
    DefaultHttpIoServer, HttpIoServer, SmallHttpIoServer, handle_http_connection,
    handle_http_connection_with_sizes,
};
