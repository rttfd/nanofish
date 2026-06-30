#![cfg_attr(not(test), no_std)]
#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

/// Logging macros
pub(crate) mod fmt;

/// HTTP protocol constants and shared utilities.
pub mod protocol;

/// HTTP client implementation and request logic.
#[cfg(feature = "embassy")]
pub mod client;
/// Error types for HTTP operations.
pub mod error;
/// HTTP request handlers and traits.
pub mod handler;
/// HTTP header types and helpers.
pub mod header;
/// Transport-generic client and server helpers.
pub mod io;
/// HTTP method enum and helpers.
pub mod method;
/// HTTP client configuration options.
pub mod options;
/// HTTP request types and parsing.
pub mod request;
/// HTTP response types and body handling.
pub mod response;
/// HTTP server implementation.
#[cfg(feature = "embassy")]
pub mod server;
/// Predefined HTTP status codes as per RFC 2616.
pub mod status_code;

#[cfg(feature = "embassy")]
pub use client::{DefaultHttpClient, HttpClient, SmallHttpClient};
pub use error::Error;
pub use handler::{HttpHandler, SimpleHandler};
pub use header::{HttpHeader, headers, mime_types};
pub use io::{
    DefaultHttpIoClient, DefaultHttpIoServer, HttpIoClient, HttpIoRequest, HttpIoServer,
    SmallHttpIoClient, SmallHttpIoServer, handle_http_connection,
    handle_http_connection_with_sizes,
};
#[cfg(feature = "tls")]
pub use io::{DefaultHttpTlsIoClient, HttpTlsIoClient, SmallHttpTlsIoClient};
pub use method::HttpMethod;
pub use options::{HttpClientOptions, TimeoutDuration};
pub use request::{HttpRequest, QueryPair, QueryPairs, QueryValues, percent_decode};
pub use response::{HttpResponse, ResponseBody};
#[cfg(feature = "embassy")]
pub use server::{DefaultHttpServer, HttpServer, ServerTimeouts, SmallHttpServer};
pub use status_code::StatusCode;
