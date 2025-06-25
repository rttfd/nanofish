#![cfg_attr(not(test), no_std)]

//! # Nanofish
//!
//! A lightweight, `no_std` HTTP client for embedded systems built on top of Embassy networking.
//!
//! Nanofish provides a simple HTTP client implementation that works on constrained environments
//! with no heap allocation, making it suitable for microcontrollers and other embedded systems.
//! It supports common HTTP methods (GET, POST, PUT, DELETE, etc.) and provides a clean API
//! for making HTTP requests.
//!
//! ## Features
//!
//! - Full `no_std` compatibility with no heap allocations
//! - Built on Embassy for async networking
//! - Support for all standard HTTP methods
//! - Automatic handling of common headers
//! - DNS resolution
//! - Timeout handling and retries
//! - Optional TLS/HTTPS support (disabled by default)
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
//! nanofish = { version = "0.3.0", features = ["tls"] }
//! ```
//!
//! ## Example
//!
//! ```no_run
//! use nanofish::{HttpClient, HttpHeader};
//! use embassy_net::Stack;
//!
//! async fn example(stack: &Stack<'_>) -> Result<(), nanofish::Error> {
//!     // Create an HTTP client with a network stack
//!     let client = HttpClient::new(stack);
//!     // Define custom headers (optional)
//!     let headers = [
//!         HttpHeader { name: "User-Agent", value: "Nanofish/0.3.0" },
//!     ];
//!     // Make a GET request
//!     let response = client.get("http://example.com/api/status", &headers).await?;
//!     // Check the response
//!     if response.status_code == 200 {
//!         // Process the response body
//!         let body = response.body;
//!     }
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
pub use header::HttpHeader;
pub use method::HttpMethod;
pub use options::HttpClientOptions;
pub use response::HttpClientResponse;

#[cfg(test)]
mod tests {
    use crate::{HttpHeader, HttpMethod};

    #[test]
    fn test_http_method_as_str() {
        assert_eq!(HttpMethod::GET.as_str(), "GET");
        assert_eq!(HttpMethod::POST.as_str(), "POST");
        assert_eq!(HttpMethod::PUT.as_str(), "PUT");
        assert_eq!(HttpMethod::DELETE.as_str(), "DELETE");
    }

    #[test]
    fn test_http_header_creation() {
        let header = HttpHeader {
            name: "Content-Type",
            value: "application/json",
        };
        assert_eq!(header.name, "Content-Type");
        assert_eq!(header.value, "application/json");
    }
}
