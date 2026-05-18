use crate::{
    error::Error,
    header::HttpHeader,
    method::HttpMethod,
    protocol::{self, DOUBLE_CRLF_LEN, MAX_HEADERS},
};
use heapless::Vec;

/// HTTP request parsed from client
#[derive(Debug)]
pub struct HttpRequest<'a> {
    /// HTTP method
    pub method: HttpMethod,
    /// Request path
    pub path: &'a str,
    /// HTTP version (e.g., "HTTP/1.1")
    pub version: &'a str,
    /// Request headers
    pub headers: Vec<HttpHeader<'a>, MAX_HEADERS>,
    /// Request body (if present)
    pub body: &'a [u8],
}

impl<'a> HttpRequest<'a> {
    /// Parse an HTTP request from headers string and body bytes
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The request line is missing or malformed
    /// - The HTTP method is invalid or unsupported  
    /// - Required parts (method, path, version) are missing
    /// - Too many headers are provided (exceeds `MAX_HEADERS`)
    pub fn parse_from(headers_str: &'a str, body: &'a [u8]) -> Result<Self, Error> {
        let mut lines = headers_str.lines();

        // Parse request line
        let request_line = lines
            .next()
            .ok_or(Error::InvalidResponse("Missing request line"))?;
        let mut parts = request_line.split_whitespace();

        let method_str = parts
            .next()
            .ok_or(Error::InvalidResponse("Missing method"))?;
        let path = parts.next().ok_or(Error::InvalidResponse("Missing path"))?;
        let version = parts
            .next()
            .ok_or(Error::InvalidResponse("Missing version"))?;

        let method = HttpMethod::try_from(method_str)
            .map_err(|_| Error::InvalidResponse("Unknown HTTP method"))?;

        // Parse headers
        let mut headers = Vec::new();
        for line in lines {
            if line.is_empty() {
                break;
            }

            if let Some(colon_pos) = line.find(':') {
                let name = line[..colon_pos].trim();
                let value = line[colon_pos + 1..].trim();

                let header = HttpHeader::new(name, value);
                headers
                    .push(header)
                    .map_err(|_| Error::InvalidResponse("Too many headers"))?;
            }
        }

        Ok(HttpRequest {
            method,
            path,
            version,
            headers,
            body,
        })
    }
}

impl<'a> TryFrom<&'a [u8]> for HttpRequest<'a> {
    type Error = Error;

    fn try_from(buffer: &'a [u8]) -> Result<Self, Self::Error> {
        // Find the end of headers (double CRLF)
        let end_of_headers = protocol::find_double_crlf(buffer)
            .ok_or(Error::InvalidResponse("Incomplete request headers"))?;

        // Parse the headers string
        let headers_str = core::str::from_utf8(&buffer[..end_of_headers])
            .map_err(|_| Error::InvalidResponse("Invalid UTF-8 in request"))?;

        // Body starts after the double CRLF
        let body = &buffer[end_of_headers + DOUBLE_CRLF_LEN..];

        Self::parse_from(headers_str, body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HttpMethod;

    #[test]
    fn test_parse_request_get() {
        let request_str =
            "GET /index.html HTTP/1.1\r\nHost: example.com\r\nUser-Agent: test\r\n\r\n";
        let body = b"";

        let request = HttpRequest::parse_from(request_str, body).unwrap();

        assert_eq!(request.method, HttpMethod::GET);
        assert_eq!(request.path, "/index.html");
        assert_eq!(request.version, "HTTP/1.1");
        assert_eq!(request.headers.len(), 2);
        assert_eq!(request.body, b"");
    }

    #[test]
    fn test_parse_request_post_with_body() {
        let request_str = "POST /api/data HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: 13\r\n\r\n";
        let body = b"{\"key\":\"value\"}";

        let request = HttpRequest::parse_from(request_str, body).unwrap();

        assert_eq!(request.method, HttpMethod::POST);
        assert_eq!(request.path, "/api/data");
        assert_eq!(request.version, "HTTP/1.1");
        assert_eq!(request.headers.len(), 2);
        assert_eq!(request.body, b"{\"key\":\"value\"}");

        // Check specific headers
        let content_type_header = request
            .headers
            .iter()
            .find(|h| h.name == "Content-Type")
            .unwrap();
        assert_eq!(content_type_header.value, "application/json");
    }

    #[test]
    fn test_parse_request_invalid_method() {
        let request_str = "INVALID /path HTTP/1.1\r\n\r\n";
        let body = b"";

        let result = HttpRequest::parse_from(request_str, body);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_request_missing_parts() {
        // Missing path
        let request_str = "GET HTTP/1.1\r\n\r\n";
        let body = b"";
        let result = HttpRequest::parse_from(request_str, body);
        assert!(result.is_err());

        // Missing version
        let request_str = "GET /path\r\n\r\n";
        let result = HttpRequest::parse_from(request_str, body);
        assert!(result.is_err());

        // Empty request
        let request_str = "";
        let result = HttpRequest::parse_from(request_str, body);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_request_all_http_methods() {
        let methods = [
            ("GET", HttpMethod::GET),
            ("POST", HttpMethod::POST),
            ("PUT", HttpMethod::PUT),
            ("DELETE", HttpMethod::DELETE),
            ("PATCH", HttpMethod::PATCH),
            ("HEAD", HttpMethod::HEAD),
            ("OPTIONS", HttpMethod::OPTIONS),
            ("TRACE", HttpMethod::TRACE),
            ("CONNECT", HttpMethod::CONNECT),
        ];

        for (method_str, expected_method) in &methods {
            let request_str = format!("{method_str} /path HTTP/1.1\r\n\r\n");
            let request = HttpRequest::parse_from(&request_str, b"").unwrap();
            assert_eq!(request.method, *expected_method);
        }
    }

    #[test]
    fn test_find_double_crlf() {
        use crate::protocol::find_double_crlf;

        // Normal case
        let data = b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\nBody";
        assert_eq!(find_double_crlf(data), Some(33));

        // At the beginning
        let data = b"\r\n\r\nBody";
        assert_eq!(find_double_crlf(data), Some(0));

        // At the end
        let data = b"Headers\r\n\r\n";
        assert_eq!(find_double_crlf(data), Some(7));

        // Not found
        let data = b"GET / HTTP/1.1\r\nHost: example.com\r\n";
        assert_eq!(find_double_crlf(data), None);

        // Too short
        let data = b"\r\n\r";
        assert_eq!(find_double_crlf(data), None);

        // Empty
        let data = b"";
        assert_eq!(find_double_crlf(data), None);
    }

    #[test]
    fn test_try_from_complete_request() {
        let buffer = b"GET /index.html HTTP/1.1\r\nHost: example.com\r\nUser-Agent: test\r\n\r\n";

        let request = HttpRequest::try_from(buffer.as_slice()).unwrap();

        assert_eq!(request.method, HttpMethod::GET);
        assert_eq!(request.path, "/index.html");
        assert_eq!(request.version, "HTTP/1.1");
        assert_eq!(request.headers.len(), 2);
        assert_eq!(request.body, b"");
    }

    #[test]
    fn test_try_from_request_with_body() {
        let buffer =
            b"POST /api/data HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{\"key\":\"value\"}";

        let request = HttpRequest::try_from(buffer.as_slice()).unwrap();

        assert_eq!(request.method, HttpMethod::POST);
        assert_eq!(request.path, "/api/data");
        assert_eq!(request.version, "HTTP/1.1");
        assert_eq!(request.headers.len(), 1);
        assert_eq!(request.body, b"{\"key\":\"value\"}");
    }

    #[test]
    fn test_try_from_incomplete_headers() {
        let buffer = b"GET /index.html HTTP/1.1\r\nHost: example.com\r\n";

        let result = HttpRequest::try_from(buffer.as_slice());
        assert!(result.is_err());
    }

    #[test]
    fn test_try_from_invalid_utf8() {
        // Create buffer with invalid UTF-8 in headers
        let mut buffer: Vec<u8, 128> = Vec::new();
        let _ = buffer.extend_from_slice(b"GET /index.html HTTP/1.1\r\nHost: ");
        let _ = buffer.push(0xFF); // Invalid UTF-8
        let _ = buffer.extend_from_slice(b"\r\n\r\n");

        let result = HttpRequest::try_from(buffer.as_slice());
        assert!(result.is_err());
    }
}
