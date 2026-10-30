//! Shared HTTP client codec helpers.

use crate::{
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
use heapless::{String, Vec};

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

    let headers_section = &response_str[status_line_end + CRLF_LEN..headers_end - DOUBLE_CRLF_LEN];
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

pub fn build_req<const RQ: usize>(
    method: HttpMethod,
    host: &str,
    path: &str,
    headers: &[HttpHeader<'_>],
    body: Option<&[u8]>,
) -> Result<String<RQ>, Error> {
    let mut req = String::<RQ>::new();

    try_push!(req.push_str(method.as_str()));
    try_push!(req.push_str(" "));
    try_push!(req.push_str(path));
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

    if !has_len && body.is_some() {
        try_push!(req.push_str(CONTENT_LENGTH));
        try_push!(req.push_str(HEADER_SEPARATOR));
        let mut len = String::<8>::new();
        if core::fmt::write(&mut len, format_args!("{}", body.unwrap_or_default().len())).is_err() {
            return Err(Error::BufferOverflow);
        }
        try_push!(req.push_str(&len));
        try_push!(req.push_str(CRLF_STR));
    }

    try_push!(req.push_str(CONNECTION_CLOSE_END));
    Ok(req)
}

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
fn chunked_body_complete(body: &[u8]) -> bool {
    let mut pos = 0;
    loop {
        let Some(line_len) = protocol::find_crlf(&body[pos..]) else {
            return false;
        };
        let size = core::str::from_utf8(&body[pos..pos + line_len])
            .ok()
            .and_then(|line| line.split(';').next())
            .and_then(|size| usize::from_str_radix(size.trim(), 16).ok());
        let Some(size) = size else {
            return true;
        };

        let data_start = pos + line_len + CRLF_LEN;
        if size == 0 {
            return trailers_complete(&body[data_start..]);
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
        pos = next;
    }
}

/// Whether the (possibly empty) trailer section after the last chunk has
/// arrived, i.e. the terminating empty line is present.
fn trailers_complete(mut rest: &[u8]) -> bool {
    loop {
        match protocol::find_crlf(rest) {
            None => return false,
            Some(0) => return true,
            Some(line_len) => rest = &rest[line_len + CRLF_LEN..],
        }
    }
}

/// Whether a response declares an explicit body length via `Content-Length`
/// or chunked transfer encoding.
///
/// Responses without either use connection close to delimit the body.
pub fn declares_length(data: &[u8]) -> bool {
    let headers_end = match protocol::find_double_crlf(data) {
        Some(pos) => pos + DOUBLE_CRLF_LEN,
        None => return false,
    };

    chunked(data) || content_length(&data[..headers_end]).is_some()
}

/// Extract the `Content-Length` value from raw header bytes.
pub fn content_length(header_bytes: &[u8]) -> Option<usize> {
    let headers_str = core::str::from_utf8(header_bytes).ok()?;
    protocol::find_header_value(headers_str, CONTENT_LENGTH)?
        .parse()
        .ok()
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
    use super::*;
    use crate::{ResponseBody, StatusCode};

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
