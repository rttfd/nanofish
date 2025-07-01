#![cfg_attr(not(test), no_std)]

//! # Nanofish
//!
//! A lightweight, `no_std` HTTP client for embedded systems built on top of Embassy networking
//! with **true zero-copy response handling**.
//!
//! Nanofish provides a simple HTTP client implementation that works on constrained environments
//! with no heap allocation, making it suitable for microcontrollers and other embedded systems.
//! It features **zero-copy response handling** where all response data is borrowed directly from
//! user-provided buffers, ensuring maximum memory efficiency.
//!
//! ## Key Features
//!
//! - **True Zero-Copy Response Handling** - Response data is borrowed directly from user-provided buffers with no copying
//! - **User-Controlled Memory Management** - You provide the buffer, controlling exactly how much memory is used
//! - Full `no_std` compatibility with no heap allocations
//! - Built on Embassy for async networking
//! - Support for all standard HTTP methods (GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS, TRACE, CONNECT)
//! - Intelligent response body handling (automatic text/binary detection based on Content-Type)
//! - Convenient header creation with pre-defined constants and methods
//! - Automatic handling of common headers
//! - DNS resolution
//! - Timeout handling and retries
//! - Optional TLS/HTTPS support (disabled by default)
//!
//! ## Zero-Copy Architecture
//!
//! Unlike traditional HTTP clients that copy response data multiple times, Nanofish uses a zero-copy approach:
//!
//! - **You control the buffer size** - Provide a buffer as large or small as needed for your use case
//! - **Direct memory references** - Response body contains direct references to data in your buffer
//! - **No hidden allocations** - All memory usage is explicit and controlled by you
//! - **Optimal for embedded** - Perfect for memory-constrained environments
//!
//! ## Feature Flags
//!
//! - `tls`: Enables TLS/HTTPS support via `embedded-tls`. When disabled (default), only HTTP
//!   requests are supported and HTTPS requests will return an error.
//!
//! To enable TLS support:
//!
//! ```toml
//! [dependencies]
//! nanofish = { version = "0.5.0", features = ["tls"] }
//! ```
//!
//! ## Example
//!
//! ```no_run
//! use nanofish::{HttpClient, HttpHeader, ResponseBody, headers, mime_types};
//! use embassy_net::Stack;
//!
//! async fn example(stack: &Stack<'_>) -> Result<(), nanofish::Error> {
//!     // Create an HTTP client with a network stack
//!     let client = HttpClient::new(stack);
//!     
//!     // You control the buffer size - make it as large or small as needed!
//!     let mut response_buffer = [0u8; 8192]; // 8KB buffer for this example
//!     
//!     // Define headers using convenience methods
//!     let headers = [
//!         HttpHeader::user_agent("Nanofish/0.5.0"),
//!         HttpHeader::content_type(mime_types::JSON),
//!         HttpHeader::authorization("Bearer token123"),
//!     ];
//!     
//!     // Or create headers manually for custom needs
//!     let custom_headers = [
//!         HttpHeader { name: "X-Custom-Header", value: "custom-value" },
//!         HttpHeader::new(headers::ACCEPT, mime_types::JSON),
//!     ];
//!     
//!     // Make a GET request with zero-copy response handling
//!     let (response, bytes_read) = client.get(
//!         "http://example.com/api/status",
//!         &headers,
//!         &mut response_buffer  // Your buffer - no hidden allocations!
//!     ).await?;
//!     
//!     println!("Read {} bytes into buffer", bytes_read);
//!     
//!     // Check if the request was successful
//!     if response.is_success() {
//!         // Handle different body types - all data references your buffer directly!
//!         match &response.body {
//!             ResponseBody::Text(text) => {
//!                 // text is a &str referencing data in your response_buffer
//!                 println!("Received text: {}", text);
//!             }
//!             ResponseBody::Binary(bytes) => {
//!                 // bytes is a &[u8] referencing data in your response_buffer
//!                 println!("Received {} bytes of binary data", bytes.len());
//!             }
//!             ResponseBody::Empty => {
//!                 println!("Empty response body");
//!             }
//!         }
//!     }
//!     
//!     Ok(())
//! }
//! ```

mod client;
mod error;
mod header;
mod method;
mod options;
mod response;

pub use client::HttpClient;
pub use error::Error;
pub use header::{HttpHeader, headers, mime_types};
pub use method::HttpMethod;
pub use options::HttpClientOptions;
pub use response::{HttpResponse, ResponseBody};
