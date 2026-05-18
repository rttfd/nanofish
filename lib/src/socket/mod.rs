#![doc = include_str!("./README.md")]
/// Socket pool implementation for managing multiple socket connections.
mod socket;

/// Implementations for various socket types.
#[cfg(feature = "embassy_impl")]
pub mod embassy_impl;

/// Test mocks for read/write streams and related utilities.
#[cfg(all(any(test, feature = "mocks"), not(feature = "embassy_impl")))]
pub mod mocks;
/// Tokio-specific adapters and wrappers for the socket traits.
#[cfg(feature = "tokio_impl")]
pub mod tokio_impl;

/// Re-export of the socket module for easier access to socket traits and types.
pub use socket::*;
