use crate::{HttpHeader, StatusCode};
use heapless::Vec;

/// HTTP Response body that can handle both text and binary data using zero-copy references
#[derive(Debug)]
pub enum ResponseBody<'a> {
    /// Text content (UTF-8 encoded) - borrowed from the response buffer
    Text(&'a str),
    /// Binary content (raw bytes) - borrowed from the response buffer
    Binary(&'a [u8]),
    /// Empty body (e.g., for HEAD requests or 204 No Content)
    Empty,
}

impl ResponseBody<'_> {
    /// Try to get the body as a UTF-8 string
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            ResponseBody::Text(s) => Some(s),
            ResponseBody::Binary(bytes) => core::str::from_utf8(bytes).ok(),
            ResponseBody::Empty => Some(""),
        }
    }

    /// Get the body as raw bytes
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            ResponseBody::Text(s) => s.as_bytes(),
            ResponseBody::Binary(bytes) => bytes,
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
/// either text or binary data using zero-copy references.
pub struct HttpResponse<'a> {
    /// The HTTP status code (e.g., 200 for OK, 404 for Not Found)
    pub status_code: StatusCode,
    /// A collection of response headers with both names and values
    pub headers: Vec<HttpHeader<'a>, 16>,
    /// The response body that can handle both text and binary data
    pub body: ResponseBody<'a>,
}

impl HttpResponse<'_> {
    /// Get a header value by name (case-insensitive)
    #[must_use]
    pub fn get_header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value)
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
        self.status_code.is_success()
    }

    /// Check if the response is a client error (4xx status codes)
    #[must_use]
    pub fn is_client_error(&self) -> bool {
        self.status_code.is_client_error()
    }

    /// Check if the response is a server error (5xx status codes)
    #[must_use]
    pub fn is_server_error(&self) -> bool {
        self.status_code.is_server_error()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::HttpHeader;
    use heapless::Vec;

    #[test]
    fn test_response_body_as_str_and_bytes() {
        let text = ResponseBody::Text("hello");
        assert_eq!(text.as_str(), Some("hello"));
        assert_eq!(text.as_bytes(), b"hello");
        let bin = ResponseBody::Binary(b"bin");
        assert_eq!(bin.as_str(), Some("bin"));
        assert_eq!(bin.as_bytes(), b"bin");
        let empty = ResponseBody::Empty;
        assert_eq!(empty.as_str(), Some(""));
        assert_eq!(empty.as_bytes(), b"");
    }

    #[test]
    fn test_response_body_is_empty_and_len() {
        let text = ResponseBody::Text("");
        assert!(text.is_empty());
        assert_eq!(text.len(), 0);
        let bin = ResponseBody::Binary(b"");
        assert!(bin.is_empty());
        assert_eq!(bin.len(), 0);
        let nonempty = ResponseBody::Text("abc");
        assert!(!nonempty.is_empty());
        assert_eq!(nonempty.len(), 3);
    }

    #[test]
    fn test_http_response_get_header() {
        let mut headers: Vec<HttpHeader, 16> = Vec::new();
        headers
            .push(HttpHeader {
                name: "Content-Type",
                value: "text/plain",
            })
            .unwrap();
        let resp = HttpResponse {
            status_code: StatusCode::Ok,
            headers,
            body: ResponseBody::Empty,
        };
        assert_eq!(resp.get_header("content-type"), Some("text/plain"));
        assert_eq!(resp.get_header("missing"), None);
    }
}
