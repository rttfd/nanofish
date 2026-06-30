//! Transport-generic client and server helpers built on `embedded-io-async`.
//!
//! This module is available without the `embassy` feature. It lets callers run
//! Nanofish over any already-connected async stream, while platform-specific
//! DNS, TCP accept/connect, and timeout behavior remain outside the core crate.

use crate::{
    error::Error,
    handler::HttpHandler,
    header::{HttpHeader, headers::CONTENT_LENGTH, headers::CONTENT_TYPE, mime_types},
    method::HttpMethod,
    options::HttpClientOptions,
    protocol::{
        self, CHUNKED, CHUNKED_END_MARKER, CONNECTION_CLOSE_END, CRLF_LEN, CRLF_STR,
        DOUBLE_CRLF_LEN, HEADER_SEPARATOR, HTTP_VERSION_LINE_SUFFIX, MAX_HEADERS,
        TRANSFER_ENCODING,
    },
    request::HttpRequest,
    response::{HttpResponse, ResponseBody},
    status_code::StatusCode,
};
use embedded_io_async::{Read, Write};
use heapless::{String, Vec};

const DEFAULT_REQUEST_SIZE: usize = 1024;
const DEFAULT_SERVER_REQUEST_SIZE: usize = 4096;
const DEFAULT_SERVER_RESPONSE_SIZE: usize = 4096;
const SMALL_SERVER_REQUEST_SIZE: usize = 1024;
const SMALL_SERVER_RESPONSE_SIZE: usize = 1024;

macro_rules! try_push {
    ($expr:expr) => {
        if $expr.is_err() {
            return Err(Error::BufferOverflow);
        }
    };
}

/// Request metadata for [`HttpIoClient`].
pub struct HttpIoRequest<'a> {
    /// HTTP method to send.
    pub method: HttpMethod,
    /// Host value used for the HTTP `Host` header.
    pub host: &'a str,
    /// Request target, for example `/`, `/api`, or `/api?limit=10`.
    pub path: &'a str,
    /// Additional request headers.
    pub headers: &'a [HttpHeader<'a>],
    /// Optional request body.
    pub body: Option<&'a [u8]>,
}

/// Transport-generic HTTP client for already-connected streams.
///
/// This client does not perform DNS resolution, TCP connection setup, TLS, or
/// sleeping between retries. It writes an HTTP request to the provided stream,
/// reads the response into the caller-provided buffer, and parses the response
/// with zero-copy body references.
pub struct HttpIoClient<const RQ: usize = DEFAULT_REQUEST_SIZE> {
    options: HttpClientOptions,
}

impl HttpIoClient<DEFAULT_REQUEST_SIZE> {
    /// Create a new transport-generic client with default options and buffer sizes.
    #[must_use]
    pub fn new() -> Self {
        Self {
            options: HttpClientOptions::default(),
        }
    }
}

impl<const RQ: usize> HttpIoClient<RQ> {
    /// Create a new transport-generic client with custom options.
    #[must_use]
    pub const fn with_options(options: HttpClientOptions) -> Self {
        Self { options }
    }

    /// Send one HTTP request over an already-connected stream.
    ///
    /// `host` is used for the HTTP `Host` header. `path` must be the request
    /// target, for example `/`, `/api`, or `/api?limit=10`.
    ///
    /// # Errors
    ///
    /// Returns an error if request construction fails, stream IO fails, no
    /// response is received, or the response cannot be parsed.
    pub async fn request<'b, S>(
        &self,
        stream: &mut S,
        request: HttpIoRequest<'_>,
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error>
    where
        S: Read + Write,
    {
        let http_request = build_http_request::<RQ>(
            request.method,
            request.host,
            request.path,
            request.headers,
            request.body,
        )?;

        stream
            .write_all(http_request.as_bytes())
            .await
            .map_err(|_| Error::TcpError)?;

        if let Some(body_data) = request.body {
            stream
                .write_all(body_data)
                .await
                .map_err(|_| Error::TcpError)?;
        }

        stream.flush().await.map_err(|_| Error::TcpError)?;

        let total_read = read_response(stream, response_buffer, self.options.max_retries).await?;
        let total_read = dechunk(response_buffer, total_read)?;
        let response = parse_http_response_zero_copy(&response_buffer[..total_read])?;
        Ok((response, total_read))
    }

    /// Convenience method for making a GET request over an already-connected stream.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpIoClient::request`].
    pub async fn get<'b, S>(
        &self,
        stream: &mut S,
        host: &str,
        path: &str,
        headers: &[HttpHeader<'_>],
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error>
    where
        S: Read + Write,
    {
        self.request(
            stream,
            HttpIoRequest {
                method: HttpMethod::GET,
                host,
                path,
                headers,
                body: None,
            },
            response_buffer,
        )
        .await
    }

    /// Convenience method for making a POST request over an already-connected stream.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpIoClient::request`].
    pub async fn post<'b, S>(
        &self,
        stream: &mut S,
        host: &str,
        path: &str,
        headers: &[HttpHeader<'_>],
        body: &[u8],
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error>
    where
        S: Read + Write,
    {
        self.request(
            stream,
            HttpIoRequest {
                method: HttpMethod::POST,
                host,
                path,
                headers,
                body: Some(body),
            },
            response_buffer,
        )
        .await
    }
}

impl Default for HttpIoClient<DEFAULT_REQUEST_SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

/// Type alias for `HttpIoClient` with the default request buffer size.
pub type DefaultHttpIoClient = HttpIoClient<DEFAULT_REQUEST_SIZE>;

/// Type alias for `HttpIoClient` with a smaller request buffer size.
pub type SmallHttpIoClient = HttpIoClient<SMALL_SERVER_REQUEST_SIZE>;

/// Transport-generic HTTP server for already-accepted streams.
///
/// This is the non-Embassy counterpart to the Embassy-backed server. It handles
/// one request/response cycle per call and leaves accept loops, timeouts, and
/// connection lifecycle to the caller.
pub struct HttpIoServer<
    const REQ_SIZE: usize = DEFAULT_SERVER_REQUEST_SIZE,
    const MAX_RESPONSE_SIZE: usize = DEFAULT_SERVER_RESPONSE_SIZE,
>;

impl HttpIoServer<DEFAULT_SERVER_REQUEST_SIZE, DEFAULT_SERVER_RESPONSE_SIZE> {
    /// Create a new transport-generic server with default buffer sizes.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl<const REQ_SIZE: usize, const MAX_RESPONSE_SIZE: usize>
    HttpIoServer<REQ_SIZE, MAX_RESPONSE_SIZE>
{
    /// Create a new transport-generic server with custom buffer sizes.
    #[must_use]
    pub const fn with_buffer_sizes() -> Self {
        Self
    }

    /// Handle one request/response cycle over an already-accepted stream.
    ///
    /// # Errors
    ///
    /// Returns an error if stream IO fails, request parsing fails, handler execution
    /// fails, or response serialization exceeds the configured response buffer.
    pub async fn handle_connection<S, H>(
        &self,
        stream: &mut S,
        handler: &mut H,
    ) -> Result<(), Error>
    where
        S: Read + Write,
        H: HttpHandler,
    {
        handle_http_connection_with_sizes::<S, H, REQ_SIZE, MAX_RESPONSE_SIZE>(stream, handler)
            .await
    }
}

impl Default for HttpIoServer<DEFAULT_SERVER_REQUEST_SIZE, DEFAULT_SERVER_RESPONSE_SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

/// Type alias for `HttpIoServer` with default request and response buffer sizes.
pub type DefaultHttpIoServer =
    HttpIoServer<DEFAULT_SERVER_REQUEST_SIZE, DEFAULT_SERVER_RESPONSE_SIZE>;

/// Type alias for `HttpIoServer` with smaller request and response buffer sizes.
pub type SmallHttpIoServer = HttpIoServer<SMALL_SERVER_REQUEST_SIZE, SMALL_SERVER_RESPONSE_SIZE>;

/// Handle a single HTTP server connection over a generic async stream.
///
/// This function reads one complete HTTP request, invokes `handler`, writes the
/// response, and flushes the stream. Accept loops, timeouts, and connection
/// lifecycle are intentionally left to the caller or platform adapter.
///
/// # Errors
///
/// Returns an error if stream IO fails, request parsing fails, handler execution
/// fails, or response serialization exceeds the configured response buffer.
pub async fn handle_http_connection<S, H>(stream: &mut S, handler: &mut H) -> Result<(), Error>
where
    S: Read + Write,
    H: HttpHandler,
{
    handle_http_connection_with_sizes::<
        S,
        H,
        DEFAULT_SERVER_REQUEST_SIZE,
        DEFAULT_SERVER_RESPONSE_SIZE,
    >(stream, handler)
    .await
}

/// Handle a single HTTP server connection with custom request/response buffer sizes.
///
/// # Errors
///
/// Returns an error if stream IO fails, request parsing fails, handler execution
/// fails, or response serialization exceeds the configured response buffer.
pub async fn handle_http_connection_with_sizes<
    S,
    H,
    const REQ_SIZE: usize,
    const MAX_RESPONSE_SIZE: usize,
>(
    stream: &mut S,
    handler: &mut H,
) -> Result<(), Error>
where
    S: Read + Write,
    H: HttpHandler,
{
    let mut request_buffer = [0; REQ_SIZE];
    let total_read = read_request(stream, &mut request_buffer).await?;
    if total_read == 0 {
        return Err(Error::NoResponse);
    }

    let request = HttpRequest::try_from(&request_buffer[..total_read])?;
    let response = handler.handle_request(&request).await.map_or_else(
        |_| {
            text_error_response::<MAX_RESPONSE_SIZE>(
                StatusCode::InternalServerError,
                "Internal Server Error",
            )
        },
        |response| response.build_bytes::<MAX_RESPONSE_SIZE>(),
    )?;

    stream
        .write_all(&response)
        .await
        .map_err(|_| Error::TcpError)?;
    stream.flush().await.map_err(|_| Error::TcpError)
}

async fn read_response<S>(
    stream: &mut S,
    response_buffer: &mut [u8],
    max_retries: usize,
) -> Result<usize, Error>
where
    S: Read,
{
    let mut total_read = 0;
    let mut retries = max_retries;

    while total_read < response_buffer.len() && retries > 0 {
        match stream.read(&mut response_buffer[total_read..]).await {
            Ok(0) => break,
            Ok(n) => {
                total_read += n;
                if is_response_complete(&response_buffer[..total_read]) {
                    break;
                }
            }
            Err(_) => {
                retries -= 1;
                if retries == 0 {
                    return Err(Error::TcpError);
                }
            }
        }
    }

    if total_read == 0 {
        return Err(Error::NoResponse);
    }

    Ok(total_read)
}

async fn read_request<S>(stream: &mut S, buf: &mut [u8]) -> Result<usize, Error>
where
    S: Read,
{
    let mut total_read = 0;
    let mut header_end = None;

    while total_read < buf.len() {
        let n = stream
            .read(&mut buf[total_read..])
            .await
            .map_err(|_| Error::TcpError)?;
        if n == 0 {
            break;
        }
        total_read += n;

        if header_end.is_none() {
            header_end = protocol::find_double_crlf(&buf[..total_read]);
        }

        if let Some(hdr_end) = header_end {
            let body_start = hdr_end + DOUBLE_CRLF_LEN;
            if let Some(content_length) = parse_content_length(&buf[..hdr_end]) {
                if total_read >= body_start + content_length {
                    break;
                }
            } else {
                break;
            }
        }
    }

    Ok(total_read)
}

fn parse_content_length(header_bytes: &[u8]) -> Option<usize> {
    let headers_str = core::str::from_utf8(header_bytes).ok()?;
    protocol::find_header_value(headers_str, CONTENT_LENGTH)?
        .parse()
        .ok()
}

fn text_error_response<const MAX_RESPONSE_SIZE: usize>(
    status: StatusCode,
    body: &str,
) -> Result<Vec<u8, MAX_RESPONSE_SIZE>, Error> {
    let mut headers = Vec::new();
    let _ = headers.push(HttpHeader::content_type(mime_types::TEXT));
    let resp = HttpResponse {
        status_code: status,
        headers,
        body: ResponseBody::Text(body),
    };
    resp.build_bytes::<MAX_RESPONSE_SIZE>()
}

fn parse_http_response_zero_copy(data: &[u8]) -> Result<HttpResponse<'_>, Error> {
    let headers_end = protocol::find_double_crlf(data)
        .ok_or(Error::InvalidResponse("Invalid HTTP response format"))?
        + DOUBLE_CRLF_LEN;

    let header_bytes = &data[..headers_end];
    let response_str = core::str::from_utf8(header_bytes)
        .map_err(|_| Error::InvalidResponse("Invalid HTTP response encoding"))?;

    let status_line_end = protocol::find_crlf(header_bytes)
        .ok_or(Error::InvalidResponse("Invalid HTTP response format"))?;

    let status_line = &response_str[..status_line_end];
    let status_code_str = status_line
        .split_whitespace()
        .nth(1)
        .ok_or(Error::InvalidResponse("Invalid HTTP status line"))?;

    let status_code: StatusCode = status_code_str.try_into()?;

    let headers_section = &response_str[status_line_end + CRLF_LEN..headers_end - DOUBLE_CRLF_LEN];
    let mut headers = Vec::<HttpHeader<'_>, MAX_HEADERS>::new();

    for header_line in headers_section.split(CRLF_STR) {
        if let Some(colon_pos) = header_line.find(':') {
            let name = header_line[..colon_pos].trim();
            let value = header_line[colon_pos + 1..].trim();

            let header = HttpHeader::new(name, value);
            if headers.push(header).is_err() {
                break;
            }
        }
    }

    let body_data = if headers_end < data.len() {
        &data[headers_end..]
    } else {
        &[]
    };

    Ok(HttpResponse {
        status_code,
        headers: headers.clone(),
        body: parse_response_body(&headers, body_data),
    })
}

fn parse_response_body<'b>(headers: &[HttpHeader<'_>], body_data: &'b [u8]) -> ResponseBody<'b> {
    if body_data.is_empty() {
        return ResponseBody::Empty;
    }

    get_content_type(headers).map_or_else(
        || parse_as_text_or_binary(body_data),
        |content_type| {
            if is_text_content_type(content_type) {
                parse_as_text_or_binary(body_data)
            } else {
                ResponseBody::Binary(body_data)
            }
        },
    )
}

fn get_content_type<'h>(headers: &'h [HttpHeader<'_>]) -> Option<&'h str> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(CONTENT_TYPE))
        .map(|h| h.value)
}

fn is_text_content_type(content_type: &str) -> bool {
    content_type.starts_with("text/")
        || content_type.starts_with("application/json")
        || content_type.starts_with("application/xml")
        || content_type.starts_with("application/x-www-form-urlencoded")
}

fn parse_as_text_or_binary(body_data: &[u8]) -> ResponseBody<'_> {
    core::str::from_utf8(body_data)
        .map_or_else(|_| ResponseBody::Binary(body_data), ResponseBody::Text)
}

fn build_http_request<const RQ: usize>(
    method: HttpMethod,
    host: &str,
    path: &str,
    headers: &[HttpHeader<'_>],
    body: Option<&[u8]>,
) -> Result<String<RQ>, Error> {
    let mut http_request = String::<RQ>::new();

    try_push!(http_request.push_str(method.as_str()));
    try_push!(http_request.push_str(" "));
    try_push!(http_request.push_str(path));
    try_push!(http_request.push_str(HTTP_VERSION_LINE_SUFFIX));
    try_push!(http_request.push_str("Host: "));
    try_push!(http_request.push_str(host));
    try_push!(http_request.push_str(CRLF_STR));

    let mut content_length_present = false;

    for header in headers {
        try_push!(http_request.push_str(header.name));
        try_push!(http_request.push_str(HEADER_SEPARATOR));
        try_push!(http_request.push_str(header.value));
        try_push!(http_request.push_str(CRLF_STR));

        if header.name.eq_ignore_ascii_case(CONTENT_LENGTH) {
            content_length_present = true;
        }
    }

    if !content_length_present && body.is_some() {
        try_push!(http_request.push_str(CONTENT_LENGTH));
        try_push!(http_request.push_str(HEADER_SEPARATOR));
        let mut len_str = String::<8>::new();
        if core::fmt::write(
            &mut len_str,
            format_args!("{}", body.unwrap_or_default().len()),
        )
        .is_err()
        {
            return Err(Error::BufferOverflow);
        }
        try_push!(http_request.push_str(&len_str));
        try_push!(http_request.push_str(CRLF_STR));
    }

    try_push!(http_request.push_str(CONNECTION_CLOSE_END));

    Ok(http_request)
}

fn is_response_complete(data: &[u8]) -> bool {
    if protocol::find_double_crlf(data).is_none() {
        return false;
    }

    if has_chunked_transfer_encoding(data) {
        return data
            .windows(CHUNKED_END_MARKER.len())
            .any(|w| w == CHUNKED_END_MARKER);
    }

    let headers_end = match protocol::find_double_crlf(data) {
        Some(pos) => pos + DOUBLE_CRLF_LEN,
        None => return true,
    };
    let header_bytes = &data[..headers_end];
    if let Ok(headers_str) = core::str::from_utf8(header_bytes)
        && let Some(value) = protocol::find_header_value(headers_str, CONTENT_LENGTH)
        && let Ok(content_length) = value.parse::<usize>()
    {
        let body_received = data.len().saturating_sub(headers_end);
        return body_received >= content_length;
    }

    false
}

fn has_chunked_transfer_encoding(data: &[u8]) -> bool {
    let headers_end = match protocol::find_double_crlf(data) {
        Some(pos) => pos + DOUBLE_CRLF_LEN,
        None => return false,
    };

    let header_bytes = &data[..headers_end];
    if let Ok(headers_str) = core::str::from_utf8(header_bytes)
        && let Some(value) = protocol::find_header_value(headers_str, TRANSFER_ENCODING)
    {
        return value.eq_ignore_ascii_case(CHUNKED);
    }
    false
}

fn dechunk(buffer: &mut [u8], total_read: usize) -> Result<usize, Error> {
    let data = &buffer[..total_read];

    if !has_chunked_transfer_encoding(data) {
        return Ok(total_read);
    }

    let headers_end = protocol::find_double_crlf(data)
        .ok_or(Error::InvalidResponse("Invalid HTTP response format"))?
        + DOUBLE_CRLF_LEN;

    let mut read_pos = headers_end;
    let mut write_pos = headers_end;

    while read_pos < total_read {
        let chunk_line_end = match protocol::find_crlf(&buffer[read_pos..total_read]) {
            Some(pos) => read_pos + pos,
            None => break,
        };

        let chunk_size_str = match core::str::from_utf8(&buffer[read_pos..chunk_line_end]) {
            Ok(s) => s.trim(),
            Err(_) => return Err(Error::InvalidResponse("Invalid chunk size encoding")),
        };

        let size_part = chunk_size_str.split(';').next().unwrap_or("0").trim();
        let chunk_size = usize::from_str_radix(size_part, 16)
            .map_err(|_| Error::InvalidResponse("Invalid chunk size"))?;

        if chunk_size == 0 {
            break;
        }

        let chunk_data_start = chunk_line_end + CRLF_LEN;
        let chunk_data_end = chunk_data_start + chunk_size;

        if chunk_data_end > total_read {
            return Err(Error::InvalidResponse("Incomplete chunked body"));
        }

        if write_pos != chunk_data_start {
            buffer.copy_within(chunk_data_start..chunk_data_end, write_pos);
        }
        write_pos += chunk_size;
        read_pos = chunk_data_end + CRLF_LEN;
    }

    Ok(write_pos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ResponseBody, SimpleHandler};
    use core::convert::Infallible;
    use embedded_io_async::ErrorType;

    struct MockStream<const IN: usize, const OUT: usize> {
        input: Vec<u8, IN>,
        output: Vec<u8, OUT>,
        read_pos: usize,
    }

    impl<const IN: usize, const OUT: usize> MockStream<IN, OUT> {
        fn new(input: &[u8]) -> Self {
            Self {
                input: Vec::from_slice(input).unwrap(),
                output: Vec::new(),
                read_pos: 0,
            }
        }
    }

    impl<const IN: usize, const OUT: usize> ErrorType for MockStream<IN, OUT> {
        type Error = Infallible;
    }

    impl<const IN: usize, const OUT: usize> Read for MockStream<IN, OUT> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            if self.read_pos >= self.input.len() {
                return Ok(0);
            }
            let n = buf.len().min(self.input.len() - self.read_pos);
            buf[..n].copy_from_slice(&self.input[self.read_pos..self.read_pos + n]);
            self.read_pos += n;
            Ok(n)
        }
    }

    impl<const IN: usize, const OUT: usize> Write for MockStream<IN, OUT> {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            self.output.extend_from_slice(buf).unwrap();
            Ok(buf.len())
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[test]
    fn test_io_client_get() {
        let response =
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nhello";
        let mut stream = MockStream::<128, 256>::new(response);
        let client = HttpIoClient::new();
        let mut buffer = [0; 128];

        let (response, _) = futures_lite::future::block_on(client.get(
            &mut stream,
            "example.com",
            "/hello",
            &[],
            &mut buffer,
        ))
        .unwrap();

        assert_eq!(response.status_code, StatusCode::Ok);
        assert_eq!(response.body.as_str(), Some("hello"));
        let request = core::str::from_utf8(&stream.output).unwrap();
        assert!(request.starts_with("GET /hello HTTP/1.1\r\nHost: example.com\r\n"));
    }

    #[test]
    fn test_handle_http_connection() {
        let request = b"GET /health HTTP/1.1\r\nHost: example.com\r\n\r\n";
        let mut stream = MockStream::<128, 512>::new(request);
        let mut handler = SimpleHandler;

        futures_lite::future::block_on(handle_http_connection_with_sizes::<_, _, 128, 512>(
            &mut stream,
            &mut handler,
        ))
        .unwrap();

        let response = core::str::from_utf8(&stream.output).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.contains("{\"status\":\"ok\"}"));
    }

    #[test]
    fn test_io_server_handle_connection() {
        let request = b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\n";
        let mut stream = MockStream::<128, 512>::new(request);
        let mut handler = SimpleHandler;
        let server = HttpIoServer::<128, 512>::with_buffer_sizes();

        futures_lite::future::block_on(server.handle_connection(&mut stream, &mut handler))
            .unwrap();

        let response = core::str::from_utf8(&stream.output).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.contains("Hello from nanofish HTTP server"));
    }

    #[test]
    fn test_io_client_binary_response() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 3\r\n\r\n\x01\x02\x03";
        let mut stream = MockStream::<128, 256>::new(response);
        let client = HttpIoClient::new();
        let mut buffer = [0; 128];

        let (response, _) = futures_lite::future::block_on(client.get(
            &mut stream,
            "example.com",
            "/bin",
            &[],
            &mut buffer,
        ))
        .unwrap();

        assert_eq!(response.body, ResponseBody::Binary(&[1, 2, 3]));
    }
}
