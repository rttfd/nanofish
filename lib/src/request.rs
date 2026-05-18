use crate::error::Error;
use crate::header::HttpHeader;
use crate::method::HttpMethod;
use heapless::Vec;

use crate::abstarct_socket::socket::{SocketRead, SocketReadWith};
use crate::http_header_parser::HttpHeaderParser;
use defmt_or_log as log;
use prefix_arena::PrefixArena;

/// Maximum number of headers allowed in a request
pub const MAX_HEADERS: usize = 16;

/// HTTP request parsed from client
#[derive(Debug)]
#[defmt_or_log::maybe_derive_format]
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

    /// WebSocket key if this is a WebSocket upgrade request
    #[cfg(feature = "ws")]
    pub web_socket_key: Option<&'a str>,
}

impl<'a> HttpRequest<'a> {
    const fn new(method: HttpMethod, path: &'a str, version: &'a str) -> Self {
        Self {
            method,
            path,
            version,
            headers: Vec::new(),
            body: &[],
            #[cfg(feature = "ws")]
            web_socket_key: None,
        }
    }

    /// Try to parse an HTTP request from a TCP stream asynchronously
    /// ## Errors
    /// Returns an error if:
    /// - Reading from the stream fails
    /// - The request is malformed
    ///
    pub async fn try_parse_from_stream<'alloc, 'buf, Reader>(
        stream: &'_ mut Reader,
        allocator: &'alloc mut PrefixArena<'buf>,
    ) -> Result<HttpRequest<'buf>, Error>
    where
        Reader: SocketReadWith + SocketRead,
        'buf: 'alloc,
    {
        let parser = HttpHeaderParser::new(stream);

        let (first_line, mut parser) = parser.parse_first_line(allocator).await.map_err(|e| {
            log::error!("Failed to parse HTTP request first line: {:?}", e);
            Error::MalformedRequest("Failed to parse HTTP request first line")
        })?;
        let mut request = HttpRequest::new(first_line.method, first_line.path, first_line.version);

        #[cfg(feature = "ws")]
        let mut web_socket_search = WebSocketKeySearch::new();
        let mut content_length_search = ContentLengthSearch::new();

        {
            while let Some(header) = parser.parse_next_header(allocator).await.map_err(|e| {
                log::error!("Failed to parse HTTP header: {:?}", e);
                Error::MalformedRequest("Failed to parse HTTP header")
            })? {
                #[cfg(feature = "ws")]
                let is_filtered_out = { content_length_search.process(&header)? || web_socket_search.process(&header) };
                #[cfg(not(feature = "ws"))]
                let is_filtered_out = content_length_search.process(&header)?;

                if is_filtered_out {
                    continue;
                }

                request
                    .headers
                    .push(header)
                    .map_err(|_| Error::MalformedRequest("Too many headers"))?;
            }
        }

        #[cfg(feature = "ws")]
        {
            request.web_socket_key = web_socket_search.web_socket_key();
        }

        // Finalize the parser (e.g., read body if needed)
        parser.finalize(allocator).await.map_err(|e| {
            log::error!("Failed to finalize HTTP parser: {:?}", e);
            Error::MalformedRequest("Failed to finalize HTTP parser")
        })?;

        let body_size = content_length_search.content_length().unwrap_or(0);

        if allocator.len() < body_size {
            // Not enough buffer to receive body
            return Err(Error::MemoryOverflow);
        }

        let read_buffer = unsafe { allocator.take_prefix_unchecked(body_size) };
        stream.read_exact(read_buffer).await.map_err(|e| {
            log::error!("Failed to read HTTP request body: {:?}", log::Debug2Format(&e));
            Error::MalformedRequest("Failed to read HTTP request body")
        })?;
        request.body = read_buffer;

        Ok(request)
    }
}

struct ContentLengthSearch {
    length: Option<usize>,
}

impl ContentLengthSearch {
    /// Create a new ContentLengthSearch
    pub const fn new() -> Self {
        Self { length: None }
    }

    /// Process a header to check for Content-Length
    ///
    /// ## Returns
    /// - Returns Ok(true) if the header was Content-Length and processed
    /// - Returns Ok(false) if the header was not Content-Length
    ///
    /// ## Errors
    /// - Returns `Error::MalformedRequest` if multiple Content-Length headers are found or if
    ///   the value is invalid.
    pub fn process(&mut self, header: &HttpHeader<'_>) -> Result<bool, Error> {
        let mut res: bool = false;

        if header.name.eq_ignore_ascii_case("Content-Length") {
            if self.length.is_none() {
                let val = header
                    .value
                    .parse::<usize>()
                    .map_err(|_| Error::MalformedRequest("Invalid Content-Length header value"))?;
                self.length = Some(val);
                res = true;
            } else {
                return Err(Error::MalformedRequest("Multiple Content-Length headers found"));
            }
        }

        Ok(res)
    }

    pub fn content_length(&self) -> Option<usize> {
        self.length
    }
}

#[cfg(feature = "ws")]
struct WebSocketKeySearch<'buf> {
    is_done: bool,
    is_upgrade: bool,
    is_connection_upgrade: bool,
    key: Option<&'buf str>,
}
#[cfg(feature = "ws")]
impl<'buf> WebSocketKeySearch<'buf> {
    /// Create a new WebSocketKeySearch
    pub const fn new() -> Self {
        Self {
            is_done: false,
            is_upgrade: false,
            is_connection_upgrade: false,
            key: None,
        }
    }

    /// Process a header to check for WebSocket upgrade headers
    /// ## Returns
    /// - Returns Some(key) if the WebSocket key is found and all upgrade headers are present
    /// - Returns None otherwise
    pub fn process(&mut self, header: &HttpHeader<'buf>) -> bool {
        let mut res: bool = false;
        if self.is_done {
            return res;
        }

        if self.key.is_none() && header.name.eq_ignore_ascii_case("Sec-WebSocket-Key") {
            self.key = Some(header.value);
            res = true;
        } else if !self.is_upgrade
            && header.name.eq_ignore_ascii_case("Upgrade")
            && header.value.eq_ignore_ascii_case("websocket")
        {
            self.is_upgrade = true;
            res = true;
        } else if !self.is_connection_upgrade
            && header.name.eq_ignore_ascii_case("Connection")
            && header.value.eq_ignore_ascii_case("upgrade")
        {
            self.is_connection_upgrade = true;
            res = true;
        }

        self.is_done = self.is_upgrade && self.is_connection_upgrade && self.key.is_some();

        res
    }

    pub fn web_socket_key(&self) -> Option<&'buf str> {
        if self.is_done { self.key } else { None }
    }
}

#[cfg(all(test, not(feature = "embassy_impl")))]
mod tests {
    use super::*;
    use crate::abstarct_socket::mocks::mock_read_stream::MockReadStream;

    fn create_mock_stream<'buf>(data: &'buf mut [u8]) -> MockReadStream<'buf> {
        MockReadStream::new(data)
    }

    #[tokio::test]
    async fn test_try_parse_from_stream() {
        let mut request = b"GET /index.html HTTP/1.1\r\nHost: example.com\r\nUser-Agent: test\r\n\r\n".to_vec();

        let mut stream = create_mock_stream(request.as_mut_slice());
        let mut buffer = [0u8; 256];
        let mut allocator = PrefixArena::new(&mut buffer);

        let request = HttpRequest::try_parse_from_stream(&mut stream, &mut allocator)
            .await
            .expect("Expected successful parse");

        assert_eq!(request.method, HttpMethod::GET);
        assert_eq!(request.path, "/index.html");
        assert_eq!(request.version, "HTTP/1.1");
        assert_eq!(request.headers.len(), 2);
        for header in request.headers {
            match header.name {
                "Host" => assert_eq!(header.value, "example.com"),
                "User-Agent" => assert_eq!(header.value, "test"),
                _ => panic!("Unexpected header"),
            }
        }
        assert_eq!(request.body, b"");
    }

    #[tokio::test]
    async fn test_try_parse_from_stream_post_with_body() {
        let mut request =
            b"POST /api/data HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: 15\r\n\r\n{\"key\":\"value\"}".to_vec();

        let mut stream = create_mock_stream(request.as_mut_slice());
        let mut buffer = [0u8; 256];
        let mut allocator = PrefixArena::new(&mut buffer);

        let request = HttpRequest::try_parse_from_stream(&mut stream, &mut allocator)
            .await
            .expect("Expected successful parse");

        assert_eq!(request.method, HttpMethod::POST);
        assert_eq!(request.path, "/api/data");
        assert_eq!(request.version, "HTTP/1.1");
        assert_eq!(request.headers.len(), 1);

        #[cfg(feature = "ws")]
        {
            assert!(request.web_socket_key.is_none());
        }

        for header in request.headers {
            match header.name {
                "Content-Type" => assert_eq!(header.value, "application/json"),
                _ => panic!("Unexpected header"),
            }
        }
        assert_eq!(request.body, b"{\"key\":\"value\"}");
    }

    #[tokio::test]
    async fn test_parse_request_invalid_method() {
        let mut request = b"INVALID /path HTTP/1.1\r\n\r\n".to_vec();

        let mut stream = create_mock_stream(request.as_mut_slice());
        let mut buffer = [0u8; 256];
        let mut allocator = PrefixArena::new(&mut buffer);

        let e = HttpRequest::try_parse_from_stream(&mut stream, &mut allocator)
            .await
            .expect_err("Expected error due to invalid method");

        assert!(matches!(e, Error::MalformedRequest(_)));
    }

    #[tokio::test]
    async fn test_parse_request_missing_version() {
        let mut request = b"GET /path\r\n\r\n".to_vec();

        let mut stream = create_mock_stream(request.as_mut_slice());
        let mut buffer = [0u8; 256];
        let mut allocator = PrefixArena::new(&mut buffer);

        let e = HttpRequest::try_parse_from_stream(&mut stream, &mut allocator)
            .await
            .expect_err("Expected error due to invalid method");

        assert!(matches!(e, Error::MalformedRequest(_)));
    }

    #[tokio::test]
    async fn test_parse_request_missing_path() {
        let mut request = b"GET  \r\n\r\n".to_vec();

        let mut stream = create_mock_stream(request.as_mut_slice());
        let mut buffer = [0u8; 256];
        let mut allocator = PrefixArena::new(&mut buffer);

        let e = HttpRequest::try_parse_from_stream(&mut stream, &mut allocator)
            .await
            .expect_err("Expected error due to invalid method");

        assert!(matches!(e, Error::MalformedRequest(_)));
    }

    #[tokio::test]
    async fn test_parse_request_missing_method() {
        let mut request = b"  \r\n\r\n".to_vec();

        let mut stream = create_mock_stream(request.as_mut_slice());
        let mut buffer = [0u8; 256];
        let mut allocator = PrefixArena::new(&mut buffer);

        let e = HttpRequest::try_parse_from_stream(&mut stream, &mut allocator)
            .await
            .expect_err("Expected error due to invalid method");

        assert!(matches!(e, Error::MalformedRequest(_)));
    }

    #[tokio::test]
    async fn test_parse_request_empty_request() {
        let mut request = b"".to_vec();

        let mut stream = create_mock_stream(request.as_mut_slice());
        let mut buffer = [0u8; 256];
        let mut allocator = PrefixArena::new(&mut buffer);

        let e = HttpRequest::try_parse_from_stream(&mut stream, &mut allocator)
            .await
            .expect_err("Expected error due to invalid method");

        assert!(matches!(e, Error::MalformedRequest(_)));
    }

    #[tokio::test]
    async fn test_parse_request_all_http_methods() {
        let methods = [
            (b"GET /path HTTP/1.1\r\n\r\n".as_slice(), HttpMethod::GET),
            (b"POST /path HTTP/1.1\r\n\r\n".as_slice(), HttpMethod::POST),
            (b"PUT /path HTTP/1.1\r\n\r\n".as_slice(), HttpMethod::PUT),
            (b"DELETE /path HTTP/1.1\r\n\r\n".as_slice(), HttpMethod::DELETE),
            (b"PATCH /path HTTP/1.1\r\n\r\n".as_slice(), HttpMethod::PATCH),
            (b"HEAD /path HTTP/1.1\r\n\r\n".as_slice(), HttpMethod::HEAD),
            (b"OPTIONS /path HTTP/1.1\r\n\r\n".as_slice(), HttpMethod::OPTIONS),
            (b"TRACE /path HTTP/1.1\r\n\r\n".as_slice(), HttpMethod::TRACE),
            (b"CONNECT /path HTTP/1.1\r\n\r\n".as_slice(), HttpMethod::CONNECT),
        ];

        for (request_bytes, expected_method) in &methods {
            let mut request_bytes = request_bytes.to_vec();

            let mut stream = create_mock_stream(request_bytes.as_mut_slice());
            let mut buffer = [0u8; 256];
            let mut allocator = PrefixArena::new(&mut buffer);

            let request = HttpRequest::try_parse_from_stream(&mut stream, &mut allocator)
                .await
                .expect("Expected successful parse");

            assert_eq!(request.method, *expected_method);
        }
    }

    #[tokio::test]
    async fn test_content_length_search() {
        let mut search = ContentLengthSearch::new();

        // Test with no Content-Length header
        let header1 = HttpHeader::new("Host", "example.com");
        assert_eq!(search.process(&header1).unwrap(), false);
        assert_eq!(search.content_length(), None);

        // Test with valid Content-Length header
        let header2 = HttpHeader::new("Content-Length", "123");
        assert_eq!(search.process(&header2).unwrap(), true);
        assert_eq!(search.content_length(), Some(123));

        let header1 = HttpHeader::new("Host", "example.com");
        assert_eq!(search.process(&header1).unwrap(), false);
        assert_eq!(search.content_length(), Some(123));

        // Test with multiple Content-Length headers
        let header3 = HttpHeader::new("Content-Length", "456");
        assert!(search.process(&header3).is_err());
    }

    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn test_web_socket_key_search() {
        let mut search = WebSocketKeySearch::new();

        // Test with no relevant headers
        let header1: HttpHeader<'_> = HttpHeader::new("Host", "example.com");
        assert_eq!(search.process(&header1), false);
        assert_eq!(search.web_socket_key(), None);

        // Test with Upgrade header only
        let header2 = HttpHeader::new("Upgrade", "websocket");
        assert_eq!(search.process(&header2), true);
        assert_eq!(search.web_socket_key(), None);

        // Test with Connection header only
        let header3 = HttpHeader::new("Connection", "upgrade");
        assert_eq!(search.process(&header3), true);
        assert_eq!(search.web_socket_key(), None);

        // Test with Sec-WebSocket-Key header only
        let header4 = HttpHeader::new("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==");
        assert_eq!(search.process(&header4), true);
        assert_eq!(search.web_socket_key(), Some("dGhlIHNhbXBsZSBub25jZQ=="));

        // Test with all headers present
        let mut search2 = WebSocketKeySearch::new();
        search2.process(&header2);
        search2.process(&header3);
        search2.process(&header4);
        assert_eq!(search2.web_socket_key(), Some("dGhlIHNhbXBsZSBub25jZQ=="));
    }
}
