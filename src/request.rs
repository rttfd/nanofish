use crate::{
    error::Error,
    header::{
        HttpHeader,
        headers::{CONTENT_LENGTH, CONTENT_TYPE, IF_NONE_MATCH},
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

    /// Whether the request's `If-None-Match` header matches `etag`, meaning the
    /// client already has the current representation and a
    /// `304 Not Modified` response is appropriate.
    ///
    /// `etag` is the quoted entity tag the server would send, for example
    /// `"\"v1\""`. Comparison is weak, as RFC 9110 requires for
    /// `If-None-Match`: a `W/` prefix on either side is ignored, and `*`
    /// matches any tag. Returns `false` when the header is absent.
    #[must_use]
    pub fn if_none_match(&self, etag: &str) -> bool {
        self.header(IF_NONE_MATCH)
            .is_some_and(|value| etag_matches(value, etag))
    }

    /// Match the request path (without its query) against a `pattern` with
    /// `:name` captures and an optional trailing `*`. See [`match_path`].
    #[must_use]
    pub fn match_path<'p, const N: usize>(
        &self,
        pattern: &'p str,
    ) -> Option<PathParams<'p, 'a, N>> {
        match_path(pattern, self.route_path())
    }

    /// Get the request body as UTF-8 text.    /// Get the request body as UTF-8 text.
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

/// Whether an `If-None-Match` header value matches the entity tag `etag`.
///
/// The header is `*` or a comma-separated list of entity tags. Tags are
/// compared weakly: a `W/` prefix is ignored on both sides.
#[must_use]
pub fn etag_matches(header_value: &str, etag: &str) -> bool {
    fn opaque(tag: &str) -> &str {
        let tag = tag.trim();
        tag.strip_prefix("W/").unwrap_or(tag)
    }

    let etag = opaque(etag);
    header_value
        .split(',')
        .map(opaque)
        .any(|candidate| candidate == "*" || (!candidate.is_empty() && candidate == etag))
}

/// Values captured by [`match_path`]: `(name, value)` pairs in pattern order.
///
/// Values are the raw path segments; use [`percent_decode`] if they may be
/// percent-encoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathParams<'p, 'a, const N: usize = 4> {
    pairs: Vec<(&'p str, &'a str), N>,
}

impl<'p, 'a, const N: usize> PathParams<'p, 'a, N> {
    /// The value captured by `:name`, or by the trailing `*` under the name
    /// `*`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&'a str> {
        self.pairs
            .iter()
            .find(|(param, _)| *param == name)
            .map(|(_, value)| *value)
    }

    /// Iterate over `(name, value)` pairs in pattern order.
    pub fn iter(&self) -> impl Iterator<Item = (&'p str, &'a str)> + '_ {
        self.pairs.iter().copied()
    }

    /// Number of captured values.
    #[must_use]
    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    /// Whether the pattern captured nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }
}

/// Match `path` (without a query string) against `pattern`.
///
/// Segments are separated by `/`. In the pattern, `:name` matches exactly one
/// non-empty segment and captures it, and a final `*` matches the rest of the
/// path (including further `/`, and possibly nothing) and is captured as `*`.
/// Every other segment must match exactly. `N` is the maximum number of
/// captures; a pattern with more yields `None`.
///
/// ```
/// use nanofish::request::match_path;
///
/// let params = match_path::<4>("/led/:id/state", "/led/3/state").unwrap();
/// assert_eq!(params.get("id"), Some("3"));
/// assert!(match_path::<4>("/led/:id/state", "/led/3").is_none());
///
/// let params = match_path::<4>("/static/*", "/static/css/site.css").unwrap();
/// assert_eq!(params.get("*"), Some("css/site.css"));
/// ```
#[must_use]
pub fn match_path<'p, 'a, const N: usize>(
    pattern: &'p str,
    path: &'a str,
) -> Option<PathParams<'p, 'a, N>> {
    let mut pairs = Vec::new();
    let mut pattern_rest = pattern;
    let mut path_rest = path;

    loop {
        let (pattern_segment, next_pattern) = split_segment(pattern_rest);
        if pattern_segment == "*" && next_pattern.is_none() {
            pairs.push(("*", path_rest)).ok()?;
            return Some(PathParams { pairs });
        }

        let (path_segment, next_path) = split_segment(path_rest);
        if let Some(name) = pattern_segment.strip_prefix(':') {
            if path_segment.is_empty() {
                return None;
            }
            pairs.push((name, path_segment)).ok()?;
        } else if pattern_segment != path_segment {
            return None;
        }

        match (next_pattern, next_path) {
            (None, None) => return Some(PathParams { pairs }),
            (Some(pattern_next), Some(path_next)) => {
                pattern_rest = pattern_next;
                path_rest = path_next;
            }
            _ => return None,
        }
    }
}

/// Split off the first `/`-separated segment, returning the rest if there was
/// a `/`.
fn split_segment(path: &str) -> (&str, Option<&str>) {
    path.split_once('/')
        .map_or((path, None), |(segment, rest)| (segment, Some(rest)))
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

    #[test]
    fn test_etag_matches() {
        assert!(etag_matches("\"v1\"", "\"v1\""));
        assert!(!etag_matches("\"v1\"", "\"v2\""));
        assert!(etag_matches("\"v0\", \"v1\" ,\"v2\"", "\"v1\""));
        // Weak comparison on either side.
        assert!(etag_matches("W/\"v1\"", "\"v1\""));
        assert!(etag_matches("\"v1\"", "W/\"v1\""));
        // `*` matches anything; an empty header matches nothing.
        assert!(etag_matches("*", "\"anything\""));
        assert!(!etag_matches("", "\"v1\""));
        assert!(!etag_matches(" , ", "\"\""));
        // Substrings are not matches.
        assert!(!etag_matches("\"v10\"", "\"v1\""));
    }

    #[test]
    fn test_request_if_none_match() {
        let raw = b"GET /x HTTP/1.1\r\nIf-None-Match: \"v1\", \"v2\"\r\n\r\n";
        let request = HttpRequest::try_from(raw.as_slice()).unwrap();
        assert!(request.if_none_match("\"v2\""));
        assert!(!request.if_none_match("\"v3\""));

        let raw = b"GET /x HTTP/1.1\r\nHost: h\r\n\r\n";
        let request = HttpRequest::try_from(raw.as_slice()).unwrap();
        assert!(!request.if_none_match("\"v1\""));
    }

    #[test]
    fn test_match_path_captures() {
        let params = match_path::<4>("/led/:id", "/led/7").unwrap();
        assert_eq!(params.get("id"), Some("7"));
        assert_eq!(params.len(), 1);

        let params = match_path::<4>("/a/:x/b/:y", "/a/1/b/2").unwrap();
        assert_eq!(params.get("x"), Some("1"));
        assert_eq!(params.get("y"), Some("2"));
        assert_eq!(params.get("z"), None);
        assert!(params.iter().eq([("x", "1"), ("y", "2")]));

        // A literal-only pattern captures nothing.
        let params = match_path::<4>("/health", "/health").unwrap();
        assert!(params.is_empty());
        assert!(match_path::<4>("/", "/").is_some());
    }

    #[test]
    fn test_match_path_rejects_mismatches() {
        for (pattern, path) in [
            ("/led/:id", "/led"),
            ("/led/:id", "/led/"),
            ("/led/:id", "/led/1/2"),
            ("/led/:id", "/fan/1"),
            ("/led", "/led/"),
            ("/led/", "/led"),
            ("/a/b", "/a/c"),
            ("/", "/x"),
            ("/x", "/"),
        ] {
            assert!(
                match_path::<4>(pattern, path).is_none(),
                "{pattern} vs {path}"
            );
        }
    }

    #[test]
    fn test_match_path_wildcard() {
        let params = match_path::<4>("/static/*", "/static/css/site.css").unwrap();
        assert_eq!(params.get("*"), Some("css/site.css"));
        let params = match_path::<4>("/static/*", "/static/").unwrap();
        assert_eq!(params.get("*"), Some(""));
        let params = match_path::<4>("/f/:name/*", "/f/a/b/c").unwrap();
        assert_eq!(params.get("name"), Some("a"));
        assert_eq!(params.get("*"), Some("b/c"));
        assert!(match_path::<4>("/static/*", "/other/x").is_none());
        // `*` is only a wildcard as the last segment.
        assert!(match_path::<4>("/*/x", "/a/x").is_none());
    }

    #[test]
    fn test_match_path_capture_capacity() {
        assert!(match_path::<1>("/:a/:b", "/1/2").is_none());
        assert!(match_path::<2>("/:a/:b", "/1/2").is_some());
    }

    #[test]
    fn test_request_match_path_ignores_query() {
        let raw = b"GET /led/3?fade=1 HTTP/1.1\r\nHost: h\r\n\r\n";
        let request = HttpRequest::try_from(raw.as_slice()).unwrap();
        let params = request.match_path::<2>("/led/:id").unwrap();
        assert_eq!(params.get("id"), Some("3"));
    }
}
