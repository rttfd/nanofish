#![cfg_attr(not(test), no_std)]
#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

/// Shared HTTP codec helpers.
pub(crate) mod codec;
/// Logging macros
pub(crate) mod fmt;

/// HTTP protocol constants and shared utilities.
pub mod protocol;

/// HTTP client ports and adapters.
pub mod client;
/// Error types for HTTP operations.
pub mod error;
/// HTTP request handlers and traits.
pub mod handler;
/// HTTP header types and helpers.
pub mod header;
/// HTTP method enum and helpers.
pub mod method;
/// HTTP client configuration options.
pub mod options;
/// HTTP request types and parsing.
pub mod request;
/// HTTP response types and body handling.
pub mod response;
/// HTTP server ports and adapters.
pub mod server;
/// smoltcp adapters for transport-generic IO APIs.
#[cfg(feature = "smoltcp")]
pub mod smoltcp;
/// Predefined HTTP status codes as per RFC 2616.
pub mod status_code;

#[cfg(feature = "embassy")]
pub use client::{DefaultEmbassyHttpClient, EmbassyHttpClient, SmallEmbassyHttpClient};
pub use client::{DefaultHttpClient, HttpClient, HttpClientRequest, SmallHttpClient};
#[cfg(feature = "tls")]
pub use client::{DefaultHttpTlsClient, HttpTlsClient, SmallHttpTlsClient, TlsVerification};
pub use client::{HttpEndpoint, parse_endpoint};
pub use error::Error;
pub use handler::{HttpHandler, SimpleHandler};
pub use header::{HttpHeader, headers, mime_types};
pub use method::HttpMethod;
pub use options::{HttpClientOptions, TimeoutDuration};
pub use request::{HttpRequest, QueryPair, QueryPairs, QueryValues, percent_decode};
pub use response::{HttpResponse, ResponseBody};
#[cfg(feature = "embassy")]
pub use server::{
    DefaultEmbassyHttpServer, EmbassyHttpServer, ServerTimeouts, SmallEmbassyHttpServer,
};
pub use server::{
    DefaultHttpServer, HttpServer, SmallHttpServer, handle_http_connection,
    handle_http_connection_with_sizes,
};
#[cfg(feature = "smoltcp")]
pub use smoltcp::{SmolTcpError, SmolTcpStream};
pub use status_code::StatusCode;
