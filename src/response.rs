use crate::{
    HttpHeader, StatusCode,
    error::Error,
    header::headers::{CONTENT_LENGTH, CONTENT_TYPE},
    protocol::{CRLF, HEADER_SEPARATOR, HTTP_VERSION_PREFIX, MAX_HEADERS},
};
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

/// HTTP Response struct with status code, headers and body
///
/// This struct represents the response received from an HTTP server.
/// It contains the status code, headers, and the response body which can be
/// either text or binary data using zero-copy references.
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
    pub fn get_header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value)
    }

    /// Get the Content-Type header value
    #[must_use]
    pub fn content_type(&self) -> Option<&str> {
        self.get_header(CONTENT_TYPE)
    }

    /// Get the Content-Length header value as a number
    #[must_use]
    pub fn content_length(&self) -> Option<usize> {
        self.get_header(CONTENT_LENGTH)?.parse().ok()
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

    /// Build HTTP response bytes from this `HttpResponse`
    ///
    /// # Errors
    ///
    /// Returns `Error::BufferOverflow` if the response exceeds `MAX_RESPONSE_SIZE`.
    pub fn build_bytes<const MAX_RESPONSE_SIZE: usize>(
        &self,
    ) -> Result<Vec<u8, MAX_RESPONSE_SIZE>, Error> {
        let mut bytes = Vec::new();

        // Status line: HTTP/1.1 <code> <reason>\r\n
        write_status_line(&mut bytes, self.status_code)?;

        // Headers
        let mut has_content_length = false;
        for header in &self.headers {
            push_slice(&mut bytes, header.name.as_bytes())?;
            push_slice(&mut bytes, HEADER_SEPARATOR.as_bytes())?;
            push_slice(&mut bytes, header.value.as_bytes())?;
            push_slice(&mut bytes, CRLF)?;
            if header.name.eq_ignore_ascii_case(CONTENT_LENGTH) {
                has_content_length = true;
            }
        }

        // Content-Length header if body is present and not already specified
        let body_bytes = self.body.as_bytes();
        if !has_content_length && !body_bytes.is_empty() {
            push_slice(&mut bytes, CONTENT_LENGTH.as_bytes())?;
            push_slice(&mut bytes, HEADER_SEPARATOR.as_bytes())?;
            write_decimal_to_buffer(&mut bytes, body_bytes.len())?;
            push_slice(&mut bytes, CRLF)?;
        }

        // End of headers
        push_slice(&mut bytes, CRLF)?;

        // Body
        push_slice(&mut bytes, body_bytes)?;

        Ok(bytes)
    }
}

/// Push a byte slice into the buffer, returning `BufferOverflow` on failure.
fn push_slice<const N: usize>(bytes: &mut Vec<u8, N>, data: &[u8]) -> Result<(), Error> {
    bytes
        .extend_from_slice(data)
        .map_err(|_| Error::BufferOverflow)
}

/// Write HTTP status line to the given buffer
fn write_status_line<const N: usize>(
    bytes: &mut Vec<u8, N>,
    status_code: StatusCode,
) -> Result<(), Error> {
    push_slice(bytes, HTTP_VERSION_PREFIX)?;
    write_decimal_to_buffer(bytes, status_code.as_u16() as usize)?;
    bytes.push(b' ').map_err(|_| Error::BufferOverflow)?;
    push_slice(bytes, status_code.text().as_bytes())?;
    push_slice(bytes, CRLF)
}

/// Write a decimal number to the buffer
fn write_decimal_to_buffer<const N: usize>(
    bytes: &mut Vec<u8, N>,
    mut num: usize,
) -> Result<(), Error> {
    if num == 0 {
        return bytes.push(b'0').map_err(|_| Error::BufferOverflow);
    }

    let mut digits = [0u8; 10];
    let mut i = 0;

    while num > 0 {
        #[allow(clippy::cast_possible_truncation)]
        {
            digits[i] = (num % 10) as u8 + b'0';
        }
        num /= 10;
        i += 1;
    }

    for j in (0..i).rev() {
        bytes.push(digits[j]).map_err(|_| Error::BufferOverflow)?;
    }
    Ok(())
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

    #[test]
    fn test_build_http_response_ok() {
        let mut headers = Vec::new();
        let _ = headers.push(HttpHeader::new("Content-Type", "text/html"));
        let _ = headers.push(HttpHeader::new("Content-Length", "12"));

        let response = HttpResponse {
            status_code: StatusCode::Ok,
            headers,
            body: ResponseBody::Text("Hello World!"),
        };

        let bytes = response.build_bytes::<4096>().unwrap();
        let response_str = core::str::from_utf8(&bytes).unwrap();

        assert!(response_str.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response_str.contains("Content-Type: text/html\r\n"));
        assert!(response_str.contains("Content-Length: 12\r\n"));
        assert!(response_str.ends_with("Hello World!"));
    }

    #[test]
    fn test_build_http_response_not_found() {
        let response = HttpResponse {
            status_code: StatusCode::NotFound,
            headers: Vec::new(),
            body: ResponseBody::Text("Not Found"),
        };

        let bytes = response.build_bytes::<4096>().unwrap();
        let response_str = core::str::from_utf8(&bytes).unwrap();

        assert!(response_str.starts_with("HTTP/1.1 404 Not Found\r\n"));
        assert!(response_str.contains("Content-Length: 9\r\n"));
        assert!(response_str.ends_with("Not Found"));
    }

    #[test]
    fn test_build_http_response_empty_body() {
        let response = HttpResponse {
            status_code: StatusCode::NoContent,
            headers: Vec::new(),
            body: ResponseBody::Empty,
        };

        let bytes = response.build_bytes::<4096>().unwrap();
        let response_str = core::str::from_utf8(&bytes).unwrap();

        assert!(response_str.starts_with("HTTP/1.1 204 No Content\r\n"));
        assert!(!response_str.contains("Content-Length"));
        assert!(response_str.ends_with("\r\n\r\n"));
    }

    #[test]
    fn test_build_http_response_binary_body() {
        let binary_data = b"\x00\x01\x02\x03";
        let response = HttpResponse {
            status_code: StatusCode::Ok,
            headers: Vec::new(),
            body: ResponseBody::Binary(binary_data),
        };

        let bytes = response.build_bytes::<4096>().unwrap();

        // Check that the response contains the binary data at the end
        assert!(bytes.ends_with(binary_data));

        // Check that content-length is correct
        let response_str = core::str::from_utf8(&bytes[..bytes.len() - binary_data.len()]).unwrap();
        assert!(response_str.contains("Content-Length: 4\r\n"));
    }

    #[test]
    fn test_write_decimal_to_buffer() {
        let mut bytes: Vec<u8, 64> = Vec::new();

        // Test zero
        write_decimal_to_buffer(&mut bytes, 0).unwrap();
        assert_eq!(bytes, b"0");

        // Test single digit
        bytes.clear();
        write_decimal_to_buffer(&mut bytes, 5).unwrap();
        assert_eq!(bytes, b"5");

        // Test multi-digit numbers
        bytes.clear();
        write_decimal_to_buffer(&mut bytes, 42).unwrap();
        assert_eq!(bytes, b"42");

        bytes.clear();
        write_decimal_to_buffer(&mut bytes, 123).unwrap();
        assert_eq!(bytes, b"123");

        bytes.clear();
        write_decimal_to_buffer(&mut bytes, 9999).unwrap();
        assert_eq!(bytes, b"9999");
    }

    #[test]
    fn test_write_status_line() {
        let mut bytes: Vec<u8, 64> = Vec::new();

        // Test common status codes
        write_status_line(&mut bytes, StatusCode::Ok).unwrap();
        assert_eq!(bytes, b"HTTP/1.1 200 OK\r\n");

        bytes.clear();
        write_status_line(&mut bytes, StatusCode::NotFound).unwrap();
        assert_eq!(bytes, b"HTTP/1.1 404 Not Found\r\n");

        bytes.clear();
        write_status_line(&mut bytes, StatusCode::InternalServerError).unwrap();
        assert_eq!(bytes, b"HTTP/1.1 500 Internal Server Error\r\n");

        bytes.clear();
        write_status_line(&mut bytes, StatusCode::Created).unwrap();
        assert_eq!(bytes, b"HTTP/1.1 201 Created\r\n");
    }

    #[test]
    fn test_content_length_calculation() {
        // Test various body lengths
        let long_text_a = "A".repeat(100);
        let long_text_b = "B".repeat(999);
        let test_cases = [
            ("", 0),
            ("a", 1),
            ("hello", 5),
            ("0123456789", 10),
            ("Lorem ipsum dolor sit amet", 26),
            (long_text_a.as_str(), 100),
            (long_text_b.as_str(), 999),
        ];

        for (body_text, expected_len) in &test_cases {
            let response = HttpResponse {
                status_code: StatusCode::Ok,
                headers: Vec::new(),
                body: ResponseBody::Text(body_text),
            };

            let bytes = response.build_bytes::<4096>().unwrap();
            let response_str = core::str::from_utf8(&bytes).unwrap();

            if *expected_len > 0 {
                let expected_header = format!("Content-Length: {expected_len}\r\n");
                assert!(
                    response_str.contains(&expected_header),
                    "Expected '{expected_header}' in response for body length {expected_len}"
                );
            }
        }
    }
}
