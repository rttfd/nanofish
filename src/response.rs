use heapless::Vec;
use nanofish::{
    HttpHeader, StatusCode,
    header::headers::{CONTENT_LENGTH, CONTENT_TYPE},
    protocol::MAX_HEADERS,
};

/// HTTP Response body that can handle both text and binary data using zero-copy references
#[derive(Debug, Clone, PartialEq, Eq, Copy)]
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
            Self::Text(s) => Some(s),
            Self::Binary(bytes) => core::str::from_utf8(bytes).ok(),
            Self::Empty => None,
        }
    }

    /// Get the body as raw bytes
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Text(s) => s.as_bytes(),
            Self::Binary(bytes) => bytes,
            Self::Empty => &[],
        }
    }

    /// Check if the body is empty
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        match self {
            Self::Text(s) => s.is_empty(),
            Self::Binary(bytes) => bytes.is_empty(),
            Self::Empty => true,
        }
    }

    /// Get the length of the body in bytes
    #[must_use]
    pub const fn len(&self) -> usize {
        match self {
            Self::Text(s) => s.len(),
            Self::Binary(bytes) => bytes.len(),
            Self::Empty => 0,
        }
    }
}

/// A response received by the client: status code, headers and body.
///
/// The body borrows the client's response buffer (zero-copy) and is told
/// apart as text or binary. Responses a server sends are
/// [`ServerResponse`](nanofish::ServerResponse)s instead.
#[derive(Debug)]
pub struct HttpResponse<'a> {
    /// The HTTP status code (e.g., 200 for OK, 404 for Not Found)
    pub status_code: StatusCode,
    /// A collection of response headers with both names and values
    pub headers: Vec<HttpHeader<'a>, MAX_HEADERS>,
    /// The response body that can handle both text and binary data
    pub body: ResponseBody<'a>,
}

impl HttpResponse<'_> {
    /// Get a header value by name (case-insensitive)
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value)
    }

    /// Get the Content-Type header value
    #[must_use]
    pub fn content_type(&self) -> Option<&str> {
        self.header(CONTENT_TYPE)
    }

    /// Get the Content-Length header value as a number
    #[must_use]
    pub fn content_length(&self) -> Option<usize> {
        self.header(CONTENT_LENGTH)?.parse().ok()
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
    use heapless::Vec;
    use nanofish::header::HttpHeader;
    use nanofish::response::*;

    #[test]
    fn test_response_body_as_str_and_bytes() {
        let text = ResponseBody::Text("hello");
        assert_eq!(text.as_str(), Some("hello"));
        assert_eq!(text.as_bytes(), b"hello");
        let bin = ResponseBody::Binary(b"bin");
        assert_eq!(bin.as_str(), Some("bin"));
        assert_eq!(bin.as_bytes(), b"bin");
        let empty = ResponseBody::Empty;
        assert_eq!(empty.as_str(), None);
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
    fn test_http_response_header() {
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
        assert_eq!(resp.header("content-type"), Some("text/plain"));
        assert_eq!(resp.header("missing"), None);
    }
}
