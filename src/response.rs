use crate::header::OwnedHttpHeader;
use heapless::{String, Vec};

/// HTTP Response body that can handle both text and binary data
#[derive(Debug)]
pub enum ResponseBody {
    /// Text content (UTF-8 encoded)
    Text(String<2048>),
    /// Binary content (raw bytes)
    Binary(Vec<u8, 2048>),
    /// Empty body (e.g., for HEAD requests or 204 No Content)
    Empty,
}

impl ResponseBody {
    /// Try to get the body as a UTF-8 string
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            ResponseBody::Text(s) => Some(s.as_str()),
            ResponseBody::Binary(bytes) => core::str::from_utf8(bytes).ok(),
            ResponseBody::Empty => Some(""),
        }
    }

    /// Get the body as raw bytes
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            ResponseBody::Text(s) => s.as_bytes(),
            ResponseBody::Binary(bytes) => bytes.as_slice(),
            ResponseBody::Empty => &[],
        }
    }

    /// Check if the body is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            ResponseBody::Text(s) => s.is_empty(),
            ResponseBody::Binary(bytes) => bytes.is_empty(),
            ResponseBody::Empty => true,
        }
    }

    /// Get the length of the body in bytes
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            ResponseBody::Text(s) => s.len(),
            ResponseBody::Binary(bytes) => bytes.len(),
            ResponseBody::Empty => 0,
        }
    }
}

/// HTTP Response struct with status code, headers and body
///
/// This struct represents the response received from an HTTP server.
/// It contains the status code, headers, and the response body which can be
/// either text or binary data.
pub struct HttpResponse {
    /// The HTTP status code (e.g., 200 for OK, 404 for Not Found)
    pub status_code: u16,
    /// A collection of response headers with both names and values
    pub headers: Vec<OwnedHttpHeader, 16>,
    /// The response body that can handle both text and binary data
    pub body: ResponseBody,
}

impl HttpResponse {
    /// Get a header value by name (case-insensitive)
    pub fn get_header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|h| h.name().eq_ignore_ascii_case(name))
            .map(super::header::OwnedHttpHeader::value)
    }

    /// Get the Content-Type header value
    #[must_use]
    pub fn content_type(&self) -> Option<&str> {
        self.get_header("Content-Type")
    }

    /// Get the Content-Length header value as a number
    #[must_use]
    pub fn content_length(&self) -> Option<usize> {
        self.get_header("Content-Length")?.parse().ok()
    }

    /// Check if the response indicates success (2xx status codes)
    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status_code)
    }

    /// Check if the response is a client error (4xx status codes)
    #[must_use]
    pub fn is_client_error(&self) -> bool {
        (400..500).contains(&self.status_code)
    }

    /// Check if the response is a server error (5xx status codes)
    #[must_use]
    pub fn is_server_error(&self) -> bool {
        (500..600).contains(&self.status_code)
    }
}
