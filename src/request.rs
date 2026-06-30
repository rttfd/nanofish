use crate::{
    error::Error,
    header::{
        HttpHeader,
        headers::{CONTENT_LENGTH, CONTENT_TYPE},
    },
    method::HttpMethod,
    protocol::{self, DOUBLE_CRLF_LEN, MAX_HEADERS},
};
use heapless::Vec;

/// A single raw query parameter pair.
///
/// Names and values are returned exactly as they appear in the URL, without
/// percent-decoding. Duplicate names are preserved and bracket syntax such as
/// `f[0]` is treated as a literal name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryPair<'a> {
    /// Raw query parameter name.
    pub name: &'a str,
    /// Raw query parameter value, or an empty string for parameters without `=`.
    pub value: &'a str,
}

/// Iterator over raw query parameter pairs.
#[derive(Debug, Clone)]
pub struct QueryPairs<'a> {
    remaining: &'a str,
}

impl<'a> Iterator for QueryPairs<'a> {
    type Item = QueryPair<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let (segment, remaining) = self.remaining.split_once('&').map_or_else(
                || (self.remaining, ""),
                |(segment, remaining)| (segment, remaining),
            );
            self.remaining = remaining;

            if segment.is_empty() {
                if self.remaining.is_empty() {
                    return None;
                }
                continue;
            }

            let (name, value) = segment
                .split_once('=')
                .map_or((segment, ""), |(name, value)| (name, value));

            return Some(QueryPair { name, value });
        }
    }
}

/// Iterator over raw values for all query parameters matching a name.
#[derive(Debug, Clone)]
pub struct QueryValues<'a, 'n> {
    pairs: QueryPairs<'a>,
    name: &'n str,
}

impl<'a> Iterator for QueryValues<'a, '_> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        self.pairs
            .find(|pair| pair.name == self.name)
            .map(|pair| pair.value)
    }
}

/// HTTP request parsed from client
#[derive(Debug)]
pub struct HttpRequest<'a> {
    /// HTTP method
    pub method: HttpMethod,
    /// Raw request target from the request line, including any query string
    pub path: &'a str,
    /// HTTP version (e.g., "HTTP/1.1")
    pub version: &'a str,
    /// Request headers
    pub headers: Vec<HttpHeader<'a>, MAX_HEADERS>,
    /// Request body (if present)
    pub body: &'a [u8],
}

impl<'a> HttpRequest<'a> {
    /// Get the raw request target from the request line, including any query string.
    #[must_use]
    pub const fn target(&self) -> &'a str {
        self.path
    }

    /// Get the path without a query string.
    #[must_use]
    pub fn route_path(&self) -> &'a str {
        self.path
            .split_once('?')
            .map_or(self.path, |(path, _query)| path)
    }

    /// Get the raw query string without the leading `?`.
    #[must_use]
    pub fn query_string(&self) -> Option<&'a str> {
        self.path.split_once('?').map(|(_path, query)| query)
    }

    /// Iterate over raw query parameter pairs.
    ///
    /// Duplicate names are preserved. Percent-decoding is not applied.
    #[must_use]
    pub fn query_pairs(&self) -> QueryPairs<'a> {
        QueryPairs {
            remaining: self.query_string().unwrap_or(""),
        }
    }

    /// Return the first raw query parameter value matching `name`.
    #[must_use]
    pub fn query(&self, name: &str) -> Option<&'a str> {
        self.query_first(name)
    }

    /// Return the first raw query parameter value matching `name`.
    #[must_use]
    pub fn query_first(&self, name: &str) -> Option<&'a str> {
        self.query_pairs()
            .find(|pair| pair.name == name)
            .map(|pair| pair.value)
    }

    /// Return the last raw query parameter value matching `name`.
    #[must_use]
    pub fn query_last(&self, name: &str) -> Option<&'a str> {
        self.query_pairs()
            .filter(|pair| pair.name == name)
            .map(|pair| pair.value)
            .last()
    }

    /// Iterate over all raw values for query parameters matching `name`.
    #[must_use]
    pub fn query_all<'n>(&self, name: &'n str) -> QueryValues<'a, 'n> {
        QueryValues {
            pairs: self.query_pairs(),
            name,
        }
    }

    /// Return the first raw query parameter value matching bracket-index syntax.
    ///
    /// For example, `query_indexed("f", 0)` matches `f[0]` and returns its
    /// value. Bracket keys remain literal for [`Self::query`]; this is only a
    /// convenience helper for applications that use indexed query names.
    #[must_use]
    pub fn query_indexed(&self, name: &str, index: usize) -> Option<&'a str> {
        self.query_pairs()
            .find(|pair| query_name_matches_index(pair.name, name, index))
            .map(|pair| pair.value)
    }

    /// Alias for [`Self::query`].
    #[must_use]
    pub fn query_param(&self, name: &str) -> Option<&'a str> {
        self.query(name)
    }

    /// Find a request header value by name, case-insensitively.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&'a str> {
        self.headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value)
    }

    /// Get the request body as UTF-8 text.
    ///
    /// # Errors
    ///
    /// Returns `Error::InvalidResponse` if the body is not valid UTF-8.
    pub fn body_str(&self) -> Result<&'a str, Error> {
        core::str::from_utf8(self.body)
            .map_err(|_| Error::InvalidResponse("Invalid UTF-8 in request body"))
    }

    /// Get the `Content-Type` header value.
    #[must_use]
    pub fn content_type(&self) -> Option<&'a str> {
        self.header(CONTENT_TYPE)
    }

    /// Get the `Content-Length` header value parsed as a number.
    #[must_use]
    pub fn content_length(&self) -> Option<usize> {
        self.header(CONTENT_LENGTH)?.parse().ok()
    }

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

/// Percent-decode an `application/x-www-form-urlencoded` query component.
///
/// The decoded output is written into `out` and returned as `&str`. `+` is
/// decoded to a space, and `%HH` sequences are decoded as bytes. The decoded
/// bytes must be valid UTF-8.
///
/// # Errors
///
/// Returns `Error::BufferOverflow` when `out` is too small, or
/// `Error::InvalidResponse` for malformed percent escapes or invalid UTF-8.
pub fn percent_decode<'a>(input: &str, out: &'a mut [u8]) -> Result<&'a str, Error> {
    let mut written = 0;
    let mut bytes = input.as_bytes().iter().copied();

    while let Some(byte) = bytes.next() {
        let decoded = match byte {
            b'+' => b' ',
            b'%' => {
                let hi = bytes
                    .next()
                    .ok_or(Error::InvalidResponse("Incomplete percent escape"))?;
                let lo = bytes
                    .next()
                    .ok_or(Error::InvalidResponse("Incomplete percent escape"))?;
                (hex_value(hi).ok_or(Error::InvalidResponse("Invalid percent escape"))? << 4)
                    | hex_value(lo).ok_or(Error::InvalidResponse("Invalid percent escape"))?
            }
            byte => byte,
        };

        let slot = out.get_mut(written).ok_or(Error::BufferOverflow)?;
        *slot = decoded;
        written += 1;
    }

    core::str::from_utf8(&out[..written])
        .map_err(|_| Error::InvalidResponse("Invalid UTF-8 in percent-decoded value"))
}

fn query_name_matches_index(query_name: &str, name: &str, index: usize) -> bool {
    let Some(rest) = query_name.strip_prefix(name) else {
        return false;
    };
    let Some(index_str) = rest
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    else {
        return false;
    };
    index_str.parse::<usize>().ok() == Some(index)
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
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
    fn test_request_query_helpers() {
        let request_str =
            "GET /search?q=rust&page=1&flag&a=&a=2 HTTP/1.1\r\nHost: example.com\r\n\r\n";
        let request = HttpRequest::parse_from(request_str, b"").unwrap();

        assert_eq!(request.target(), "/search?q=rust&page=1&flag&a=&a=2");
        assert_eq!(request.route_path(), "/search");
        assert_eq!(request.query_string(), Some("q=rust&page=1&flag&a=&a=2"));
        assert_eq!(request.query("q"), Some("rust"));
        assert_eq!(request.query_param("page"), Some("1"));
        assert_eq!(request.query("flag"), Some(""));
        assert_eq!(request.query_first("a"), Some(""));
        assert_eq!(request.query_last("a"), Some("2"));
        assert_eq!(request.query("missing"), None);

        let mut values = request.query_all("a");
        assert_eq!(values.next(), Some(""));
        assert_eq!(values.next(), Some("2"));
        assert_eq!(values.next(), None);
    }

    #[test]
    fn test_request_query_duplicate_and_bracket_keys() {
        let request_str = "GET /items?a=1&a=2&f[0]=1&f[1]=2 HTTP/1.1\r\n\r\n";
        let request = HttpRequest::parse_from(request_str, b"").unwrap();

        assert_eq!(request.query_first("a"), Some("1"));
        assert_eq!(request.query_last("a"), Some("2"));
        assert_eq!(request.query("f[0]"), Some("1"));
        assert_eq!(request.query("f[1]"), Some("2"));
        assert_eq!(request.query("f"), None);
        assert_eq!(request.query_indexed("f", 0), Some("1"));
        assert_eq!(request.query_indexed("f", 1), Some("2"));
        assert_eq!(request.query_indexed("f", 2), None);

        let mut pairs = request.query_pairs();
        assert_eq!(
            pairs.next(),
            Some(QueryPair {
                name: "a",
                value: "1"
            })
        );
        assert_eq!(
            pairs.next(),
            Some(QueryPair {
                name: "a",
                value: "2"
            })
        );
    }

    #[test]
    fn test_request_header_and_body_helpers() {
        let request_str =
            "POST /submit HTTP/1.1\r\ncontent-type: text/plain\r\nContent-Length: 5\r\n\r\n";
        let request = HttpRequest::parse_from(request_str, b"hello").unwrap();

        assert_eq!(request.header("Content-Type"), Some("text/plain"));
        assert_eq!(request.content_type(), Some("text/plain"));
        assert_eq!(request.content_length(), Some(5));
        assert_eq!(request.body_str().unwrap(), "hello");
    }

    #[test]
    fn test_percent_decode_query_component() {
        let mut out = [0u8; 32];
        assert_eq!(
            percent_decode("hello+world%21%2F", &mut out).unwrap(),
            "hello world!/"
        );
        assert!(percent_decode("bad%", &mut out).is_err());
        assert!(percent_decode("bad%xx", &mut out).is_err());
        assert!(percent_decode("toolong", &mut [0u8; 3]).is_err());
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
