//! HTTP protocol constants and shared utilities.

/// Carriage Return + Line Feed (bytes)
pub const CRLF: &[u8] = b"\r\n";

/// Carriage Return + Line Feed (string)
pub const CRLF_STR: &str = "\r\n";

/// Length of CRLF
pub const CRLF_LEN: usize = CRLF.len();

/// Double CRLF — separates HTTP headers from body (bytes)
pub const DOUBLE_CRLF: &[u8] = b"\r\n\r\n";

/// Double CRLF (string)
pub const DOUBLE_CRLF_STR: &str = "\r\n\r\n";

/// Length of double CRLF
pub const DOUBLE_CRLF_LEN: usize = DOUBLE_CRLF.len();

/// Chunked transfer encoding final chunk marker
pub const CHUNKED_END_MARKER: &[u8] = b"0\r\n\r\n";

/// HTTP/1.1 version string
pub const HTTP_VERSION: &str = "HTTP/1.1";

/// HTTP/1.1 version bytes with leading space
pub const HTTP_VERSION_PREFIX: &[u8] = b"HTTP/1.1 ";

/// HTTP/1.1 version as string with leading space and trailing CRLF (for request line)
pub const HTTP_VERSION_LINE_SUFFIX: &str = " HTTP/1.1\r\n";

/// Default HTTP port
pub const DEFAULT_HTTP_PORT: u16 = 80;

/// Default HTTPS port
pub const DEFAULT_HTTPS_PORT: u16 = 443;

/// Transfer-Encoding header name
pub const TRANSFER_ENCODING: &str = "Transfer-Encoding";

/// Chunked transfer encoding value
pub const CHUNKED: &str = "chunked";

/// Header-value separator
pub const HEADER_SEPARATOR: &str = ": ";

/// Connection close header line with trailing CRLF and end-of-headers CRLF
pub const CONNECTION_CLOSE_END: &str = "Connection: close\r\n\r\n";

/// Maximum number of headers allowed in requests and responses
pub const MAX_HEADERS: usize = 16;

/// Find the position of the double CRLF sequence in raw bytes.
/// Returns the byte index of the start of `\r\n\r\n`.
#[must_use]
pub fn find_double_crlf(data: &[u8]) -> Option<usize> {
    data.windows(DOUBLE_CRLF_LEN).position(|w| w == DOUBLE_CRLF)
}

/// Find the position of CRLF in raw bytes starting from a given slice.
/// Returns the byte index relative to the start of the provided slice.
#[must_use]
pub fn find_crlf(data: &[u8]) -> Option<usize> {
    data.windows(CRLF_LEN).position(|w| w == CRLF)
}

/// Check if a header name matches (case-insensitive) and return its value.
#[must_use]
pub fn find_header_value<'a>(headers_str: &'a str, target_name: &str) -> Option<&'a str> {
    for line in headers_str.split(CRLF_STR) {
        if let Some(colon_pos) = line.find(':') {
            let name = line[..colon_pos].trim();
            let value = line[colon_pos + 1..].trim();
            if name.eq_ignore_ascii_case(target_name) {
                return Some(value);
            }
        }
    }
    None
}
