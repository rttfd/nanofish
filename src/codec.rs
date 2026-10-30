//! Shared HTTP client codec helpers.

use heapless::{String, Vec};
use nanofish::{
    error::Error,
    header::{HttpHeader, headers::CONTENT_LENGTH, headers::CONTENT_TYPE},
    method::HttpMethod,
    protocol::{
        self, CHUNKED, CONNECTION_CLOSE_END, CRLF_LEN, CRLF_STR, DOUBLE_CRLF_LEN, HEADER_SEPARATOR,
        HTTP_VERSION_LINE_SUFFIX, MAX_HEADERS, TRANSFER_ENCODING,
    },
    response::{HttpResponse, ResponseBody},
    status_code::StatusCode,
};

macro_rules! try_push {
    ($expr:expr) => {
        if $expr.is_err() {
            return Err(Error::BufferOverflow);
        }
    };
}

pub fn parse_resp(data: &[u8]) -> Result<HttpResponse<'_>, Error> {
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

    // With no header lines the status line's CRLF is also the start of the
    // terminating blank line, so the section is empty.
    let headers_section = response_str
        .get(status_line_end + CRLF_LEN..headers_end - DOUBLE_CRLF_LEN)
        .unwrap_or_default();
    let mut headers = Vec::<HttpHeader<'_>, MAX_HEADERS>::new();

    for header_line in headers_section.split(CRLF_STR) {
        if let Some(colon_pos) = header_line.find(':') {
            let name = header_line[..colon_pos].trim();
            let value = header_line[colon_pos + 1..].trim();
            if headers.push(HttpHeader::new(name, value)).is_err() {
                break;
            }
        }
    }

    let body_data = if headers_end < data.len() {
        &data[headers_end..]
    } else {
        &[]
    };

    let body = parse_body(&headers, body_data);

    Ok(HttpResponse {
        status_code,
        headers,
        body,
    })
}

pub fn parse_body<'b>(headers: &[HttpHeader<'_>], body_data: &'b [u8]) -> ResponseBody<'b> {
    if body_data.is_empty() {
        return ResponseBody::Empty;
    }

    content_type(headers).map_or_else(
        || text_or_bin(body_data),
        |ct| {
            if is_text(ct) {
                text_or_bin(body_data)
            } else {
                ResponseBody::Binary(body_data)
            }
        },
    )
}

/// Build the request line and headers.
///
/// The request target is `path`, followed by `?` and `query` when present.
///
/// # Errors
///
/// - [`Error::InvalidUrl`] if `path` is empty or `path`, `query` or `host`
///   contain spaces or control characters.
/// - [`Error::HeaderError`] if a header name is not a valid token or a value
///   contains control characters such as CR or LF.
/// - [`Error::BufferOverflow`] if the request does not fit in `RQ` bytes.
#[cfg(test)]
pub fn build_req<const RQ: usize>(
    method: HttpMethod,
    host: &str,
    path: &str,
    query: Option<&str>,
    headers: &[HttpHeader<'_>],
    body: Option<&[u8]>,
) -> Result<String<RQ>, Error> {
    build_req_with(method, host, path, query, headers, body, false)
}

/// Like [`build_req`]. With `keep_alive`, no `Connection: close` is sent, so an
/// HTTP/1.1 server keeps the connection open for another request.
pub fn build_req_with<const RQ: usize>(
    method: HttpMethod,
    host: &str,
    path: &str,
    query: Option<&str>,
    headers: &[HttpHeader<'_>],
    body: Option<&[u8]>,
    keep_alive: bool,
) -> Result<String<RQ>, Error> {
    // Anything that could end the request line or a header early would let
    // the caller's data inject headers or whole requests.
    if path.is_empty() || !protocol::is_valid_request_token(path) {
        return Err(Error::InvalidUrl);
    }
    if query.is_some_and(|query| !protocol::is_valid_request_token(query)) {
        return Err(Error::InvalidUrl);
    }
    if !protocol::is_valid_request_token(host) {
        return Err(Error::InvalidUrl);
    }
    for header in headers {
        if !protocol::is_valid_header_name(header.name) {
            return Err(Error::HeaderError("Invalid header name"));
        }
        if !protocol::is_valid_header_value(header.value) {
            return Err(Error::HeaderError("Invalid header value"));
        }
    }

    let mut req = String::<RQ>::new();

    try_push!(req.push_str(method.as_str()));
    try_push!(req.push_str(" "));
    try_push!(req.push_str(path));
    if let Some(query) = query {
        try_push!(req.push_str("?"));
        try_push!(req.push_str(query));
    }
    try_push!(req.push_str(HTTP_VERSION_LINE_SUFFIX));
    try_push!(req.push_str("Host: "));
    try_push!(req.push_str(host));
    try_push!(req.push_str(CRLF_STR));

    let mut has_len = false;
    for header in headers {
        try_push!(req.push_str(header.name));
        try_push!(req.push_str(HEADER_SEPARATOR));
        try_push!(req.push_str(header.value));
        try_push!(req.push_str(CRLF_STR));
        if header.name.eq_ignore_ascii_case(CONTENT_LENGTH) {
            has_len = true;
        }
    }

    // POST, PUT and PATCH without a body still need an explicit length, or
    // some servers answer `411 Length Required`.
    let carries_body = matches!(
        method,
        HttpMethod::POST | HttpMethod::PUT | HttpMethod::PATCH
    );
    if !has_len && (body.is_some() || carries_body) {
        try_push!(req.push_str(CONTENT_LENGTH));
        try_push!(req.push_str(HEADER_SEPARATOR));
        let mut len = String::<8>::new();
        if core::fmt::write(&mut len, format_args!("{}", body.unwrap_or_default().len())).is_err() {
            return Err(Error::BufferOverflow);
        }
        try_push!(req.push_str(&len));
        try_push!(req.push_str(CRLF_STR));
    }

    if keep_alive {
        try_push!(req.push_str(CRLF_STR));
    } else {
        try_push!(req.push_str(CONNECTION_CLOSE_END));
    }
    Ok(req)
}

#[cfg(test)]
pub fn complete(data: &[u8]) -> bool {
    let headers_end = match protocol::find_double_crlf(data) {
        Some(pos) => pos + DOUBLE_CRLF_LEN,
        None => return false,
    };

    if chunked(data) {
        return chunked_body_complete(&data[headers_end..]);
    }

    content_length(&data[..headers_end])
        .is_some_and(|content_length| data.len().saturating_sub(headers_end) >= content_length)
}

/// Whether a full response to a `method` request has been received.
///
/// Responses to `HEAD` requests and `1xx`, `204` and `304` responses never
/// carry a body, even when their headers advertise a `Content-Length` or
/// chunked encoding, so they are complete once the headers end. Everything else
/// follows [`complete`].
#[cfg(test)]
pub fn response_complete(method: HttpMethod, data: &[u8]) -> bool {
    if protocol::find_double_crlf(data).is_none() {
        return false;
    }
    bodyless(method, data) || complete(data)
}

/// Whether the response head in `data` belongs to a response without a body.
fn bodyless(method: HttpMethod, data: &[u8]) -> bool {
    method == HttpMethod::HEAD || matches!(status_code(data), Some(100..=199 | 204 | 304))
}

/// Numeric status code from the status line, if it is present and valid.
fn status_code(data: &[u8]) -> Option<u16> {
    let line_end = protocol::find_crlf(data)?;
    core::str::from_utf8(&data[..line_end])
        .ok()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

/// Whether `body` holds a whole chunked body: every chunk, the last (zero
/// size) chunk, any trailers and the final empty line.
///
/// The chunk framing is walked rather than searched for `0\r\n\r\n`, which can
/// also appear in chunk data or at the end of a header block (`Age: 0`).
/// Malformed framing counts as complete so that [`dechunk`] reports the error
/// instead of the read loop waiting for bytes that will never make it valid.
///
/// This stateless form is the reference for [`ChunkScan`], which the read loops
/// use to avoid rescanning a growing buffer after every read.
#[cfg(test)]
pub fn chunked_body_complete(body: &[u8]) -> bool {
    ChunkScan::default().advance(body)
}

/// Incremental version of [`chunked_body_complete`].
///
/// It remembers how far the framing was walked, so checking a buffer after
/// each read costs only the newly arrived bytes instead of rescanning the
/// whole body (quadratic for many small reads).
#[derive(Debug, Default, Clone, Copy)]
pub struct ChunkScan {
    /// Start of the first frame (chunk-size line or trailer line) not yet
    /// fully seen.
    pos: usize,
    /// Past the last chunk: only trailer lines and the final empty line remain.
    trailers: bool,
}

impl ChunkScan {
    /// How many bytes of the body [`ChunkScan::advance`] has consumed. After it
    /// returned `true` for a well-formed body, this is where the body ends: any
    /// bytes after it belong to something else.
    #[must_use]
    pub const fn end(&self) -> usize {
        self.pos
    }

    /// Walk the framing in `body` (everything after the headers, from its
    /// start) as far as it goes; `true` once the body is complete or its
    /// framing is malformed.
    pub fn advance(&mut self, body: &[u8]) -> bool {
        loop {
            let Some(line_len) = protocol::find_crlf(&body[self.pos..]) else {
                return false;
            };
            if self.trailers {
                if line_len == 0 {
                    self.pos += CRLF_LEN;
                    return true;
                }
                self.pos += line_len + CRLF_LEN;
                continue;
            }

            let size = core::str::from_utf8(&body[self.pos..self.pos + line_len])
                .ok()
                .and_then(|line| line.split(';').next())
                .and_then(|size| usize::from_str_radix(size.trim(), 16).ok());
            let Some(size) = size else {
                return true;
            };

            let data_start = self.pos + line_len + CRLF_LEN;
            if size == 0 {
                self.pos = data_start;
                self.trailers = true;
                continue;
            }

            // Chunk data is followed by its own CRLF.
            let Some(next) = data_start
                .checked_add(size)
                .and_then(|end| end.checked_add(CRLF_LEN))
            else {
                return true;
            };
            if next > body.len() {
                return false;
            }
            self.pos = next;
        }
    }
}

/// How the end of a response is determined.
#[derive(Debug, Clone, Copy)]
enum ResponseEnd {
    /// The headers are not complete yet.
    Pending,
    /// No body: complete at the end of the headers.
    Headers,
    /// A body of this many bytes.
    Length(usize),
    /// A chunked body.
    Chunked(ChunkScan),
    /// No declared length: the connection close ends the body.
    Close,
}

/// How the body of a response is delimited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyFraming {
    /// No body: the response ends with its headers.
    None,
    /// A body of exactly this many bytes.
    Length(usize),
    /// A chunked body.
    Chunked,
    /// No declared length: the connection close ends the body.
    UntilClose,
}

/// Determine how the body of the response to a `method` request is delimited,
/// from the raw response head (status line and headers, including the final
/// empty line).
pub fn response_framing(method: HttpMethod, head: &[u8]) -> BodyFraming {
    if bodyless(method, head) {
        BodyFraming::None
    } else if chunked(head) {
        BodyFraming::Chunked
    } else if let Some(length) = content_length(head) {
        BodyFraming::Length(length)
    } else {
        BodyFraming::UntilClose
    }
}

/// Tracks whether a growing response buffer holds a complete response.
///
/// [`response_complete`] re-derives everything from scratch on each call. A
/// `ResponseScan` finds the end of the headers and decides how the body is
/// delimited once, then only looks at new bytes, so it can be asked after every
/// read at constant cost per byte.
#[derive(Debug, Clone, Copy)]
pub struct ResponseScan {
    method: HttpMethod,
    /// Where to resume looking for the end of the headers.
    searched: usize,
    headers_end: usize,
    end: ResponseEnd,
}

impl ResponseScan {
    /// Start scanning the response to a `method` request.
    pub const fn new(method: HttpMethod) -> Self {
        Self {
            method,
            searched: 0,
            headers_end: 0,
            end: ResponseEnd::Pending,
        }
    }

    /// Whether `data`, everything read so far, holds a complete response.
    ///
    /// `data` must only ever grow between calls.
    pub fn is_complete(&mut self, data: &[u8]) -> bool {
        if matches!(self.end, ResponseEnd::Pending) {
            // A terminator split across reads starts up to 3 bytes back.
            let from = self.searched.saturating_sub(DOUBLE_CRLF_LEN - 1);
            match protocol::find_double_crlf(&data[from..]) {
                None => {
                    self.searched = data.len();
                    return false;
                }
                Some(pos) => {
                    self.headers_end = from + pos + DOUBLE_CRLF_LEN;
                    self.end = self.classify(data);
                }
            }
        }

        let body = &data[self.headers_end..];
        match &mut self.end {
            ResponseEnd::Pending | ResponseEnd::Close => false,
            ResponseEnd::Headers => true,
            ResponseEnd::Length(length) => body.len() >= *length,
            ResponseEnd::Chunked(scan) => scan.advance(body),
        }
    }

    /// Whether the headers declared where the body ends (a length, chunked
    /// framing, or no body). Without that, only the connection closing ends
    /// the response.
    pub const fn declares_length(&self) -> bool {
        matches!(
            self.end,
            ResponseEnd::Headers | ResponseEnd::Length(_) | ResponseEnd::Chunked(_)
        )
    }

    fn classify(&self, data: &[u8]) -> ResponseEnd {
        match response_framing(self.method, &data[..self.headers_end]) {
            BodyFraming::None => ResponseEnd::Headers,
            BodyFraming::Chunked => ResponseEnd::Chunked(ChunkScan::default()),
            BodyFraming::Length(length) => ResponseEnd::Length(length),
            BodyFraming::UntilClose => ResponseEnd::Close,
        }
    }
}

/// Whether a response declares an explicit body length via `Content-Length`
/// or chunked transfer encoding.
///
/// Responses without either use connection close to delimit the body.
#[cfg(test)]
pub fn declares_length(data: &[u8]) -> bool {
    let headers_end = match protocol::find_double_crlf(data) {
        Some(pos) => pos + DOUBLE_CRLF_LEN,
        None => return false,
    };

    chunked(data) || content_length(&data[..headers_end]).is_some()
}

/// Extract the `Content-Length` value from raw header bytes.
///
/// A missing, malformed or conflicting header yields `None`; use
/// [`check_content_length`] to tell those cases apart.
pub fn content_length(header_bytes: &[u8]) -> Option<usize> {
    check_content_length(header_bytes).ok().flatten()
}

/// Extract the `Content-Length` value from raw header bytes, rejecting
/// malformed or conflicting values.
///
/// Repeated `Content-Length` headers, or comma-separated lists, are accepted
/// only when every value is identical (RFC 9110 section 8.6). Values must be
/// plain decimal digits: signs, spaces inside the number and empty values are
/// rejected. Disagreeing lengths are a request-smuggling vector, so they are
/// an error rather than "use the first one".
///
/// # Errors
///
/// Returns a static description if a value is not a decimal number that fits
/// in `usize`, or if the values disagree.
pub fn check_content_length(header_bytes: &[u8]) -> Result<Option<usize>, &'static str> {
    let headers_str = core::str::from_utf8(header_bytes).map_err(|_| "Invalid header encoding")?;

    let mut length = None;
    for line in headers_str.split(CRLF_STR) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case(CONTENT_LENGTH) {
            continue;
        }
        for part in value.split(',') {
            let part = part.trim();
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return Err("Invalid Content-Length");
            }
            let part: usize = part.parse().map_err(|_| "Invalid Content-Length")?;
            match length {
                None => length = Some(part),
                Some(existing) if existing == part => {}
                Some(_) => return Err("Conflicting Content-Length"),
            }
        }
    }
    Ok(length)
}

/// Reject a response whose `Content-Length` is malformed or conflicting.
///
/// A chunked response is exempt: `Transfer-Encoding` takes precedence over
/// `Content-Length` for the framing of the body.
pub fn check_response_framing(data: &[u8]) -> Result<(), Error> {
    let Some(headers_end) = protocol::find_double_crlf(data) else {
        return Ok(());
    };
    if chunked(data) {
        return Ok(());
    }
    check_content_length(&data[..headers_end])
        .map(|_| ())
        .map_err(Error::InvalidResponse)
}

/// How the body of a request is delimited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framing {
    /// No body: the request ends with its headers.
    None,
    /// A body of exactly this many bytes.
    Length(usize),
    /// A chunked body.
    Chunked,
}

/// Why the framing of a request could not be determined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramingError {
    /// Malformed or contradictory framing headers (answer `400`).
    Invalid(&'static str),
    /// A transfer coding other than plain `chunked` (answer `501`).
    UnsupportedCoding,
}

/// Determine how a request body is delimited from the raw request headers
/// (without the final empty line).
///
/// `Transfer-Encoding: chunked` and `Content-Length` together are rejected,
/// since a peer and an intermediary could disagree on where the request ends.
/// Any transfer coding besides a single `chunked` is unsupported: the body
/// would not be decoded, so it is not accepted.
///
/// # Errors
///
/// See [`FramingError`].
pub fn request_framing(header_bytes: &[u8]) -> Result<Framing, FramingError> {
    let headers_str = core::str::from_utf8(header_bytes)
        .map_err(|_| FramingError::Invalid("Invalid request headers encoding"))?;
    let length = check_content_length(header_bytes).map_err(FramingError::Invalid)?;

    let Some(coding) = protocol::find_header_value(headers_str, TRANSFER_ENCODING) else {
        return Ok(length.map_or(Framing::None, Framing::Length));
    };
    let mut codings = coding.split(',').map(str::trim).filter(|c| !c.is_empty());
    match (codings.next(), codings.next()) {
        (Some(only), None) if only.eq_ignore_ascii_case(CHUNKED) => {
            if length.is_some() {
                Err(FramingError::Invalid(
                    "Both Transfer-Encoding and Content-Length",
                ))
            } else {
                Ok(Framing::Chunked)
            }
        }
        _ => Err(FramingError::UnsupportedCoding),
    }
}

pub fn dechunk(buffer: &mut [u8], total_read: usize) -> Result<usize, Error> {
    let data = &buffer[..total_read];
    if !chunked(data) {
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
        // A hostile chunk size must not overflow the offset arithmetic.
        let chunk_data_end = chunk_data_start
            .checked_add(chunk_size)
            .ok_or(Error::InvalidResponse("Invalid chunk size"))?;
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

fn content_type<'h>(headers: &'h [HttpHeader<'_>]) -> Option<&'h str> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(CONTENT_TYPE))
        .map(|h| h.value)
}

fn is_text(content_type: &str) -> bool {
    content_type.starts_with("text/")
        || content_type.starts_with("application/json")
        || content_type.starts_with("application/xml")
        || content_type.starts_with("application/x-www-form-urlencoded")
}

fn text_or_bin(body_data: &[u8]) -> ResponseBody<'_> {
    core::str::from_utf8(body_data)
        .map_or_else(|_| ResponseBody::Binary(body_data), ResponseBody::Text)
}

fn chunked(data: &[u8]) -> bool {
    let headers_end = match protocol::find_double_crlf(data) {
        Some(pos) => pos + DOUBLE_CRLF_LEN,
        None => return false,
    };

    let header_bytes = &data[..headers_end];
    if let Ok(headers_str) = core::str::from_utf8(header_bytes)
        && let Some(value) = protocol::find_header_value(headers_str, TRANSFER_ENCODING)
    {
        // `Transfer-Encoding` may list several codings, e.g. "gzip, chunked";
        // `chunked` must be the final coding and signals the chunked framing.
        return value
            .split(',')
            .any(|encoding| encoding.trim().eq_ignore_ascii_case(CHUNKED));
    }
    false
}

#[cfg(test)]
mod tests {
    use nanofish::codec::*;
    use nanofish::{ResponseBody, StatusCode};

    fn build(path: &str, host: &str, headers: &[HttpHeader<'_>]) -> Result<(), Error> {
        build_req::<256>(HttpMethod::GET, host, path, None, headers, None).map(|_| ())
    }

    #[test]
    fn test_build_req_accepts_ordinary_requests() {
        let headers = [
            HttpHeader::new("Authorization", "Bearer abc.def-ghi"),
            HttpHeader::new("X-API-Key", "k3y"),
            HttpHeader::new("Accept", "text/html,\tapplication/json;q=0.9"),
        ];
        build(
            "/api/v1/items?limit=10&q=a%20b",
            "example.com:8080",
            &headers,
        )
        .unwrap();
        build("/", "[::1]:80", &[]).unwrap();
    }

    #[test]
    fn test_build_req_rejects_path_injection() {
        for path in [
            "/x HTTP/1.1\r\nHost: evil\r\n\r\nGET /",
            "/x\r\nX-Injected: 1",
            "/x\ny",
            "/a b",
            "/a\0",
            "",
        ] {
            assert!(
                matches!(build(path, "example.com", &[]), Err(Error::InvalidUrl)),
                "{path:?}"
            );
        }
    }

    #[test]
    fn test_build_req_appends_query() {
        let req = build_req::<128>(HttpMethod::GET, "h", "/a", Some("x=1&y=2"), &[], None).unwrap();
        assert!(req.starts_with("GET /a?x=1&y=2 HTTP/1.1\r\nHost: h\r\n"));

        let req = build_req::<128>(HttpMethod::GET, "h", "/a", Some(""), &[], None).unwrap();
        assert!(req.starts_with("GET /a? HTTP/1.1\r\n"));

        let req = build_req::<128>(HttpMethod::GET, "h", "/a", None, &[], None).unwrap();
        assert!(req.starts_with("GET /a HTTP/1.1\r\n"));
    }

    #[test]
    fn test_build_req_rejects_query_injection() {
        for query in ["x=1 HTTP/1.1\r\nHost: evil", "x=1\r\nX: y", "a b", "x\0"] {
            assert!(
                matches!(
                    build_req::<128>(HttpMethod::GET, "h", "/a", Some(query), &[], None),
                    Err(Error::InvalidUrl)
                ),
                "{query:?}"
            );
        }
    }

    #[test]
    fn test_build_req_sends_content_length_zero_for_bodyless_post_put_patch() {
        for method in [HttpMethod::POST, HttpMethod::PUT, HttpMethod::PATCH] {
            let req = build_req::<128>(method, "h", "/a", None, &[], None).unwrap();
            assert!(req.contains("Content-Length: 0\r\n"), "{method:?}: {req}");
        }
        // Methods without request content send no length.
        for method in [HttpMethod::GET, HttpMethod::HEAD, HttpMethod::DELETE] {
            let req = build_req::<128>(method, "h", "/a", None, &[], None).unwrap();
            assert!(!req.contains("Content-Length"), "{method:?}: {req}");
        }
        // An explicit length is not duplicated.
        let headers = [HttpHeader::new("Content-Length", "0")];
        let req = build_req::<128>(HttpMethod::POST, "h", "/a", None, &headers, None).unwrap();
        assert_eq!(req.matches("Content-Length").count(), 1);
    }

    #[test]
    fn test_check_content_length() {
        let ok = |headers: &str| check_content_length(headers.as_bytes());
        assert_eq!(ok("Host: h"), Ok(None));
        assert_eq!(ok("Content-Length: 5"), Ok(Some(5)));
        assert_eq!(ok("content-length:   5  "), Ok(Some(5)));
        assert_eq!(ok("Host: h\r\nContent-Length: 0"), Ok(Some(0)));
        // Identical repeats and lists are fine.
        assert_eq!(ok("Content-Length: 5\r\nContent-Length: 5"), Ok(Some(5)));
        assert_eq!(ok("Content-Length: 5, 5"), Ok(Some(5)));
        // Other headers that merely contain the name are ignored.
        assert_eq!(
            ok("X-Content-Length: abc\r\nContent-Length: 7"),
            Ok(Some(7))
        );

        for bad in [
            "Content-Length: abc",
            "Content-Length: +5",
            "Content-Length: -1",
            "Content-Length: 5 5",
            "Content-Length: 0x10",
            "Content-Length: ",
            "Content-Length: 5,",
            "Content-Length: 99999999999999999999999",
        ] {
            assert_eq!(ok(bad), Err("Invalid Content-Length"), "{bad}");
        }
        for conflict in [
            "Content-Length: 5\r\nContent-Length: 6",
            "Content-Length: 5, 6",
            "Content-Length: 5\r\nX: y\r\nContent-Length: 05, 5, 7",
        ] {
            assert_eq!(
                ok(conflict),
                Err("Conflicting Content-Length"),
                "{conflict}"
            );
        }
    }

    #[test]
    fn test_content_length_is_none_when_invalid() {
        assert_eq!(content_length(b"Content-Length: 5"), Some(5));
        assert_eq!(content_length(b"Content-Length: abc"), None);
        assert_eq!(
            content_length(b"Content-Length: 5\r\nContent-Length: 6"),
            None
        );
    }

    #[test]
    fn test_check_response_framing() {
        assert!(
            check_response_framing(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello").is_ok()
        );
        assert!(check_response_framing(b"HTTP/1.1 200 OK\r\nContent-Length: 5").is_ok());
        assert!(matches!(
            check_response_framing(
                b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Length: 6\r\n\r\nhello"
            ),
            Err(Error::InvalidResponse("Conflicting Content-Length"))
        ));
        // Transfer-Encoding takes precedence over Content-Length.
        assert!(
            check_response_framing(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: abc\r\n\r\n"
            )
            .is_ok()
        );
    }

    #[test]
    fn test_request_framing() {
        let framing = |headers: &str| request_framing(headers.as_bytes());
        assert_eq!(framing("GET / HTTP/1.1\r\nHost: h"), Ok(Framing::None));
        assert_eq!(
            framing("POST / HTTP/1.1\r\nContent-Length: 5"),
            Ok(Framing::Length(5))
        );
        assert_eq!(
            framing("POST / HTTP/1.1\r\nTransfer-Encoding: chunked"),
            Ok(Framing::Chunked)
        );
        assert_eq!(
            framing("POST / HTTP/1.1\r\ntransfer-encoding: Chunked"),
            Ok(Framing::Chunked)
        );
    }

    #[test]
    fn test_request_framing_rejects_ambiguity() {
        let framing = |headers: &str| request_framing(headers.as_bytes());
        assert_eq!(
            framing("POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\nContent-Length: 5"),
            Err(FramingError::Invalid(
                "Both Transfer-Encoding and Content-Length"
            ))
        );
        assert_eq!(
            framing("POST / HTTP/1.1\r\nContent-Length: 5\r\nContent-Length: 6"),
            Err(FramingError::Invalid("Conflicting Content-Length"))
        );
        assert_eq!(
            framing("POST / HTTP/1.1\r\nContent-Length: abc"),
            Err(FramingError::Invalid("Invalid Content-Length"))
        );
        for coding in ["gzip", "gzip, chunked", "chunked, chunked", "identity", ""] {
            assert_eq!(
                framing(
                    &heapless::format!(96; "POST / HTTP/1.1\r\nTransfer-Encoding: {coding}")
                        .unwrap()
                ),
                Err(FramingError::UnsupportedCoding),
                "{coding:?}"
            );
        }
    }

    #[test]
    fn test_build_req_rejects_host_injection() {
        for host in ["example.com\r\nX-Injected: 1", "exa mple.com", "h\n"] {
            assert!(
                matches!(build("/", host, &[]), Err(Error::InvalidUrl)),
                "{host:?}"
            );
        }
    }

    #[test]
    fn test_build_req_rejects_header_injection() {
        let value = [HttpHeader::new("X-Token", "abc\r\nX-Injected: 1")];
        assert!(matches!(
            build("/", "example.com", &value),
            Err(Error::HeaderError("Invalid header value"))
        ));
        let nul = [HttpHeader::new("X-Token", "a\0b")];
        assert!(matches!(
            build("/", "example.com", &nul),
            Err(Error::HeaderError("Invalid header value"))
        ));

        for name in ["X-A: b\r\nX-B", "X Bad", "", "X-A:"] {
            let header = [HttpHeader::new(name, "v")];
            assert!(
                matches!(
                    build("/", "example.com", &header),
                    Err(Error::HeaderError("Invalid header name"))
                ),
                "{name:?}"
            );
        }
    }

    #[test]
    fn test_is_response_complete_no_content_length() {
        // Without Content-Length or chunked, response is never "complete" —
        // the read loop must rely on connection close (Ok(0))
        let data = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\n";
        assert!(!complete(data));
        assert!(!declares_length(data));
    }

    #[test]
    fn test_is_response_complete_content_length_zero() {
        // Content-Length: 0 means empty body — complete once headers end
        let data = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
        assert!(complete(data));
        assert!(declares_length(data));
    }

    #[test]
    fn test_is_response_complete_with_content_length() {
        let data = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
        assert!(complete(data));
        assert!(declares_length(data));
    }

    #[test]
    fn test_is_response_complete_incomplete() {
        let data = b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nshort";
        assert!(!complete(data));
    }

    #[test]
    fn test_is_response_complete_no_headers() {
        let data = b"HTTP/1.1 200";
        assert!(!complete(data));
    }

    #[test]
    fn test_parse_http_response_binary_body() {
        // Simulate a PNG-like response with invalid UTF-8 in the body
        let header = b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 8\r\n\r\n";
        let binary_body: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]; // PNG magic bytes
        let mut data = [0u8; 256];
        data[..header.len()].copy_from_slice(header);
        data[header.len()..header.len() + binary_body.len()].copy_from_slice(&binary_body);
        let data = &data[..header.len() + binary_body.len()];

        let response = parse_resp(data).expect("should parse binary response");

        assert_eq!(response.status_code, StatusCode::Ok);
        assert!(matches!(response.body, ResponseBody::Binary(b) if b == binary_body));
    }

    #[test]
    fn test_parse_http_response_without_headers() {
        // The status line's CRLF doubles as the start of the blank line.
        let response = parse_resp(b"HTTP/1.1 204 No Content\r\n\r\n").expect("should parse");
        assert_eq!(response.status_code, StatusCode::NoContent);
        assert!(response.headers.is_empty());
        assert_eq!(response.body, ResponseBody::Empty);

        let response = parse_resp(b"HTTP/1.0 200 OK\r\n\r\nhello").expect("should parse");
        assert_eq!(response.status_code, StatusCode::Ok);
        assert!(response.headers.is_empty());
        assert_eq!(response.body.as_str(), Some("hello"));
    }

    #[test]
    fn test_parse_http_response_text_body() {
        let data = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nhello";

        let response = parse_resp(data).expect("should parse text response");

        assert_eq!(response.status_code, StatusCode::Ok);
        assert!(matches!(response.body, ResponseBody::Text("hello")));
    }

    #[test]
    fn test_is_response_complete_chunked() {
        let incomplete = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n";
        assert!(!complete(incomplete));
        assert!(declares_length(incomplete));

        let complete_data =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
        assert!(complete(complete_data));
    }

    #[test]
    fn test_chunked_complete_header_ending_in_zero() {
        // `Age: 0\r\n\r\n` ends in the bytes `0\r\n\r\n`, but that is the end of
        // the headers, not the last chunk.
        let headers_only = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nAge: 0\r\n\r\n";
        assert!(!complete(headers_only));

        let partial =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nAge: 0\r\n\r\n5\r\nhello\r\n";
        assert!(!complete(partial));

        let full =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nAge: 0\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
        assert!(complete(full));
    }

    #[test]
    fn test_chunked_complete_marker_inside_chunk_data() {
        // The 7 data bytes `ab0\r\n\r\n` contain the last-chunk marker.
        let partial = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n7\r\nab0\r\n\r\n\r\n";
        assert!(!complete(partial));

        let full =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n7\r\nab0\r\n\r\n\r\n0\r\n\r\n";
        assert!(complete(full));

        let mut buf = [0u8; 128];
        buf[..full.len()].copy_from_slice(full);
        let new_len = dechunk(&mut buf, full.len()).expect("should decode chunked");
        let response = parse_resp(&buf[..new_len]).expect("should parse dechunked response");
        assert_eq!(response.body.as_str(), Some("ab0\r\n\r\n"));
    }

    #[test]
    fn test_chunked_complete_partial_framing() {
        let head = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";
        let cases: [&[u8]; 5] = [
            b"",                                // nothing yet
            b"5",                               // size line unfinished
            b"5\r\nhello",                      // chunk data CRLF missing
            b"5\r\nhello\r\n0\r\n",             // last chunk without terminating line
            b"5\r\nhello\r\n0\r\nX-Sum: 1\r\n", // trailer without terminating line
        ];
        for body in cases {
            let mut data = heapless::Vec::<u8, 128>::new();
            data.extend_from_slice(head).unwrap();
            data.extend_from_slice(body).unwrap();
            assert!(!complete(&data), "{body:?} should be incomplete");
        }
    }

    #[test]
    fn test_chunked_complete_with_trailers() {
        let data = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\nX-Sum: 1\r\n\r\n";
        assert!(complete(data));
    }

    #[test]
    fn test_chunked_complete_malformed_framing_is_complete() {
        // Malformed framing must not make the read loop wait: `dechunk`
        // reports the error.
        let bad_size = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\nhello\r\n";
        assert!(complete(bad_size));

        let mut buf = [0u8; 128];
        buf[..bad_size.len()].copy_from_slice(bad_size);
        assert!(matches!(
            dechunk(&mut buf, bad_size.len()),
            Err(Error::InvalidResponse("Invalid chunk size"))
        ));

        let overflow =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nffffffffffffffff\r\nhello\r\n";
        assert!(complete(overflow));
    }

    #[test]
    fn test_response_complete_head_ignores_body_framing() {
        // A HEAD response advertises the length of the body it does not send.
        let content_length = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n";
        assert!(response_complete(HttpMethod::HEAD, content_length));
        assert!(!response_complete(HttpMethod::GET, content_length));

        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";
        assert!(response_complete(HttpMethod::HEAD, chunked));
        assert!(!response_complete(HttpMethod::GET, chunked));

        // Still waits for the end of the headers.
        assert!(!response_complete(
            HttpMethod::HEAD,
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n"
        ));
    }

    #[test]
    fn test_response_complete_bodyless_statuses() {
        for status in ["100 Continue", "101 Switching Protocols", "204 No Content"] {
            let mut data = heapless::Vec::<u8, 128>::new();
            data.extend_from_slice(b"HTTP/1.1 ").unwrap();
            data.extend_from_slice(status.as_bytes()).unwrap();
            data.extend_from_slice(b"\r\n\r\n").unwrap();
            assert!(
                response_complete(HttpMethod::GET, &data),
                "{status} should be complete at the end of the headers"
            );
        }

        // A 304 may repeat the `Content-Length` of the cached representation.
        let not_modified = b"HTTP/1.1 304 Not Modified\r\nContent-Length: 5\r\n\r\n";
        assert!(response_complete(HttpMethod::GET, not_modified));
    }

    #[test]
    fn test_response_complete_other_statuses_follow_body_framing() {
        // No declared length: only connection close ends the body.
        let no_length = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nhello";
        assert!(!response_complete(HttpMethod::GET, no_length));

        let partial = b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nshort";
        assert!(!response_complete(HttpMethod::GET, partial));

        let full = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
        assert!(response_complete(HttpMethod::GET, full));
    }

    /// Feed `raw` to a scan in pieces of `step` bytes; the index of the first
    /// prefix that completes it, if any.
    fn first_complete(method: HttpMethod, raw: &[u8], step: usize) -> Option<usize> {
        let mut scan = ResponseScan::new(method);
        let mut end = 0;
        while end < raw.len() {
            end = (end + step).min(raw.len());
            if scan.is_complete(&raw[..end]) {
                return Some(end);
            }
        }
        None
    }

    #[test]
    fn test_response_scan_matches_stateless_check_for_every_split() {
        let responses: [&[u8]; 7] = [
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello",
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nAge: 0\r\n\r\n5\r\nhello\r\n0\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n7\r\nab0\r\n\r\n\r\n0\r\nX-Sum: 1\r\n\r\n",
            b"HTTP/1.1 204 No Content\r\n\r\n",
            b"HTTP/1.1 304 Not Modified\r\nContent-Length: 9\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nuntil close",
        ];
        for raw in responses {
            // The earliest prefix the stateless check accepts.
            let expected = (1..=raw.len()).find(|&n| response_complete(HttpMethod::GET, &raw[..n]));
            for step in [1, 2, 3, 5, 64] {
                let got = first_complete(HttpMethod::GET, raw, step);
                // Coarser steps can only skip past the exact prefix.
                match (expected, got) {
                    (None, None) => {}
                    (Some(e), Some(g)) => assert!(g >= e && g < e + step, "{raw:?} step {step}"),
                    other => panic!("{raw:?} step {step}: {other:?}"),
                }
            }
        }
    }

    #[test]
    fn test_response_scan_head_response_is_complete_at_headers() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n";
        assert_eq!(first_complete(HttpMethod::HEAD, raw, 1), Some(raw.len()));
        assert_eq!(first_complete(HttpMethod::GET, raw, 1), None);
    }

    #[test]
    fn test_response_scan_declares_length() {
        let mut scan = ResponseScan::new(HttpMethod::GET);
        assert!(!scan.declares_length());
        assert!(!scan.is_complete(b"HTTP/1.1 200 OK\r\nContent-Len"));
        assert!(!scan.declares_length());
        assert!(!scan.is_complete(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhe"));
        assert!(scan.declares_length());

        let mut scan = ResponseScan::new(HttpMethod::GET);
        assert!(!scan.is_complete(b"HTTP/1.1 200 OK\r\n\r\nbody"));
        assert!(!scan.declares_length());
    }

    #[test]
    fn test_response_scan_header_terminator_split_across_reads() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
        // Cut inside the final CRLFCRLF at every position.
        for cut in raw.len() - 3..raw.len() {
            let mut scan = ResponseScan::new(HttpMethod::GET);
            assert!(!scan.is_complete(&raw[..cut]));
            assert!(scan.is_complete(raw), "cut {cut}");
        }
    }

    #[test]
    fn test_chunk_scan_resumes_across_calls() {
        let body = b"3\r\nabc\r\n4;ext=1\r\nwxyz\r\n0\r\nT: v\r\n\r\n";
        for step in 1..=body.len() {
            let mut scan = ChunkScan::default();
            let mut end = 0;
            let mut done = false;
            while end < body.len() {
                end = (end + step).min(body.len());
                done = scan.advance(&body[..end]);
                if done {
                    break;
                }
            }
            assert!(done && end == body.len(), "step {step}");
        }
    }

    #[test]
    fn test_chunked_with_intermediate_encoding() {
        // `chunked` may be preceded by other codings, e.g. "gzip, chunked".
        let complete_data =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip, chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
        assert!(complete(complete_data));
        assert!(declares_length(complete_data));
    }

    #[test]
    fn test_dechunk_single_chunk() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: text/plain\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
        let mut buf = [0u8; 256];
        buf[..raw.len()].copy_from_slice(raw);

        let new_len = dechunk(&mut buf, raw.len()).expect("should decode chunked");

        let response = parse_resp(&buf[..new_len]).expect("should parse dechunked response");

        assert_eq!(response.status_code, StatusCode::Ok);
        assert_eq!(response.body.as_str(), Some("hello"));
    }

    #[test]
    fn test_dechunk_multiple_chunks() {
        // Mimics the weather API response from issue #29
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: application/json\r\n\r\nb\r\n{\"temp\":23}\r\n0\r\n\r\n";
        let mut buf = [0u8; 256];
        buf[..raw.len()].copy_from_slice(raw);

        let new_len = dechunk(&mut buf, raw.len()).expect("should decode chunked");

        let response = parse_resp(&buf[..new_len]).expect("should parse dechunked response");

        assert_eq!(response.body.as_str(), Some("{\"temp\":23}"));
    }

    #[test]
    fn test_dechunk_chunk_extensions() {
        // Chunk-size lines may carry extensions after `;`.
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5;name=value\r\nhello\r\n0\r\n\r\n";
        let mut buf = [0u8; 256];
        buf[..raw.len()].copy_from_slice(raw);

        let new_len = dechunk(&mut buf, raw.len()).expect("should decode chunked");

        let response = parse_resp(&buf[..new_len]).expect("should parse dechunked response");
        assert_eq!(response.body.as_str(), Some("hello"));
    }

    #[test]
    fn test_dechunk_with_intermediate_encoding() {
        let raw =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip, chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
        let mut buf = [0u8; 256];
        buf[..raw.len()].copy_from_slice(raw);

        let new_len = dechunk(&mut buf, raw.len()).expect("should decode chunked");

        let response = parse_resp(&buf[..new_len]).expect("should parse dechunked response");
        assert_eq!(response.body.as_str(), Some("hello"));
    }

    #[test]
    fn test_dechunk_noop_when_not_chunked() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
        let mut buf = [0u8; 128];
        buf[..raw.len()].copy_from_slice(raw);

        let new_len = dechunk(&mut buf, raw.len()).expect("should pass through");
        assert_eq!(new_len, raw.len());
    }

    #[test]
    fn test_dechunk_chunk_size_overflow_is_rejected() {
        // Sizes near `usize::MAX` must be rejected instead of overflowing the
        // chunk end offset (panic in debug, wrapped slice range in release).
        for size in ["ffffffffffffffff", "fffffffffffffff0", "7fffffffffffffff"] {
            let mut raw = heapless::Vec::<u8, 128>::new();
            raw.extend_from_slice(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                .unwrap();
            raw.extend_from_slice(size.as_bytes()).unwrap();
            raw.extend_from_slice(b"\r\nhello\r\n0\r\n\r\n").unwrap();
            let mut buf = [0u8; 128];
            buf[..raw.len()].copy_from_slice(&raw);

            let result = dechunk(&mut buf, raw.len());

            assert!(
                matches!(result, Err(Error::InvalidResponse(_))),
                "chunk size {size} should be rejected"
            );
        }
    }

    #[test]
    fn test_dechunk_incomplete_chunk_body() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhel";
        let mut buf = [0u8; 128];
        buf[..raw.len()].copy_from_slice(raw);

        let err = dechunk(&mut buf, raw.len()).expect_err("should reject incomplete chunk");
        assert!(matches!(
            err,
            Error::InvalidResponse("Incomplete chunked body")
        ));
    }
}
