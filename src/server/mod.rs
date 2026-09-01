//! HTTP server ports and adapters.

#[cfg(feature = "embassy")]
mod embassy;
mod io;

#[cfg(feature = "embassy")]
pub use embassy::{
    DefaultEmbassyHttpServer, EmbassyHttpServer, ServerTimeouts, SmallEmbassyHttpServer,
};
pub use io::{
    DefaultHttpServer, HttpServer, SmallHttpServer, handle_http_connection,
    handle_http_connection_with_sizes,
};
