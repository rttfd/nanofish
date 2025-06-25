use crate::header::HttpHeader;
use heapless::{String, Vec};

/// HTTP Response struct with status code, headers and body
///
/// This struct represents the response received from an HTTP server.
/// It contains the status code, headers, and the response body.
pub struct HttpClientResponse {
    /// The HTTP status code (e.g., 200 for OK, 404 for Not Found)
    pub status_code: u16,
    /// A collection of response headers
    pub headers: Vec<HttpHeader<'static>, 16>,
    /// The response body as a string with a maximum capacity of 2048 bytes
    pub body: String<2048>,
}
