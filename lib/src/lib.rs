#![cfg_attr(not(feature = "std"), no_std)]
#![doc = include_str!("../../README.md")]
#![warn(missing_docs)]

#[cfg(all(feature = "tokio_impl", feature = "embassy_impl"))]
compile_error!("features `tokio_impl` and `embassy_impl` are mutually exclusive");

#[cfg(all(feature = "tokio_impl", feature = "defmt"))]
compile_error!("feature `defmt` is only supported with `embassy_impl`");

#[cfg(all(feature = "tokio_impl", feature = "proto-ipv6"))]
compile_error!("feature `proto-ipv6` is only supported with `embassy_impl`");

#[cfg(all(feature = "embassy_impl", feature = "log"))]
compile_error!("feature `log` is only supported with `tokio_impl`");

#[cfg(all(test, feature = "defmt"))]
mod defmt_test_logger {
    #[defmt::global_logger]
    struct TestLogger;

    unsafe impl defmt::Logger for TestLogger {
        fn acquire() {}

        unsafe fn release() {}

        unsafe fn flush() {}

        unsafe fn write(bytes: &[u8]) {
            let _ = bytes;
        }
    }

    defmt::timestamp!("{=u8}", 0);
}

/// This module contains the implementation of the abstract socket traits and utilities,
/// providing a common interface for socket operations.
pub mod abstarct_socket;
/// HTTP header types and helpers.
mod header;

/// HTTP method enum and helpers.
mod method;

/// Stream-based HTTP request parser.
mod http_header_parser;

/// Error types for HTTP operations.
mod error;
/// Predefined HTTP status codes as per RFC 2616.
mod status_code;

/// HTTP request handlers and traits.
mod handler;
/// HTTP request types and parsing.
mod request;
/// HTTP response builder utilities.
mod response;
/// HTTP server implementation and related types.
pub mod server;

/// Common utilities and types for socket management.
mod worker_memory;

#[cfg(all(test, not(feature = "embassy_impl")))]
mod mocks;

/// This module contains the implementation of WebSocket traits and utilities, providing support for WebSocket communication in the library.
#[cfg(feature = "ws")]
mod web_socket;

mod allocator;

/// This module re-exports the main HTTP handling traits and types for easier access by users of the library.
pub mod http_handler {
    pub use crate::handler::*;

    /// This module contains the implementation of the allocator types and utilities, providing efficient memory management for
    /// HTTP request handling.
    pub use crate::allocator::*;

    pub use crate::request::HttpRequest;
    pub use crate::response::*;

    pub use crate::error::Error;

    pub use crate::method::HttpMethod;
    pub use crate::status_code::StatusCode;

    pub use crate::header::{HttpHeader, headers, mime_types};

    pub use crate::abstarct_socket::socket::SocketEndpoint;
}
