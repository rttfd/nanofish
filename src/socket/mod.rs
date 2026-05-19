#![doc = include_str!("./README.md")]
/// Socket pool implementation for managing multiple socket connections.
mod socket_traits;

/// Trait defining the behavior of a socket listener, including methods for accepting incoming connections and retrieving the local endpoint.
mod socket_listener;

/// Implementations for various socket types.
#[cfg(feature = "embassy_impl")]
pub mod embassy_impl;

/// Trait defining the behavior of a socket connector, including methods for establishing outgoing connections and retrieving the remote endpoint.
mod socket_connector;

/// Test mocks for read/write streams and related utilities.
#[cfg(all(any(test, feature = "mocks"), not(feature = "embassy_impl")))]
pub mod mocks;
/// Tokio-specific adapters and wrappers for the socket traits.
#[cfg(feature = "tokio_impl")]
pub mod tokio_impl;

/// Re-export of the socket module for easier access to socket traits and types.
pub use socket_traits::*;

/// Re-export of the socket listener module for easier access to socket listener traits and types.
pub use socket_listener::*;

/// Re-export of the socket connector module for easier access to socket connector traits and types.
pub use socket_connector::*;
