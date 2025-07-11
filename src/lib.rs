#![cfg_attr(not(test), no_std)]
#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

/// HTTP client implementation and request logic.
pub mod client;
/// Error types for HTTP operations.
pub mod error;
/// HTTP header types and helpers.
pub mod header;
/// HTTP method enum and helpers.
pub mod method;
/// HTTP client configuration options.
pub mod options;
/// HTTP response types and body handling.
pub mod response;
/// Predefined HTTP status codes as per RFC 2616.
pub mod status_code;

pub use client::{DefaultHttpClient, HttpClient, SmallHttpClient};
pub use error::Error;
pub use header::{HttpHeader, headers, mime_types};
pub use method::HttpMethod;
pub use options::HttpClientOptions;
pub use response::{HttpResponse, ResponseBody};
pub use status_code::StatusCode;
