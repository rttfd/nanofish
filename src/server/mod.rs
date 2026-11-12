//! HTTP server ports and adapters.

pub(crate) mod body;
#[cfg(feature = "embassy")]
mod embassy;
mod io;
pub(crate) mod response;
mod static_site;
mod stream;

pub use body::{BodyReader, RequestBody};
#[cfg(feature = "embassy")]
pub use embassy::{
    DefaultEmbassyHttpServer, EmbassyHttpServer, ServerTimeouts, SmallEmbassyHttpServer,
};
pub use io::{
    DefaultHttpServer, HttpServer, SmallHttpServer, handle_http_connection,
    handle_http_connection_with_sizes, serve_http_connection_with_sizes,
};
pub use response::{ServerResponse, ServerResponseBuilder};
pub use static_site::{
    FileStat, FileStore, StaticFile, StaticSite, mime_for_extension, serve, serve_with,
};
pub use stream::{
    BodyLength, ResponseWriter, StreamingHandler, handle_http_stream_connection,
    handle_http_stream_connection_with_sizes,
};
