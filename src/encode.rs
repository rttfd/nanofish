//! Allocation-free encoders for building requests.
//!
//! Percent-encoding for query strings and form bodies, Base64, and
//! `Authorization` header values. All of them write into caller-provided or fixed-capacity buffers and report
//! [`Error::BufferOverflow`] when the result does not fit.

use core::fmt::Write as _;
use heapless::String;
use nanofish::error::Error;

const HEX: &[u8; 16] = b"0123456789ABCDEF";
const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Whether `byte` may appear unescaped in a query component (RFC 3986
/// "unreserved" characters).
const fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

/// Percent-encode `input` into `out`, escaping everything except the RFC 3986
/// unreserved characters (`A-Z a-z 0-9 - . _ ~`).
///
/// The result is safe as a single query or form component, for example a value
/// that contains `&`, `=`, `#` or a space (written as `%20`). It is the inverse
/// of [`percent_decode`](nanofish::percent_decode).
///
/// # Errors
///
/// Returns [`Error::BufferOverflow`] if `out` is too small.
pub fn percent_encode<'a>(input: &str, out: &'a mut [u8]) -> Result<&'a str, Error> {
    let mut written = 0;
    let mut put = |byte: u8| -> Result<(), Error> {
        *out.get_mut(written).ok_or(Error::BufferOverflow)? = byte;
        written += 1;
        Ok(())
    };

    for byte in input.bytes() {
        if is_unreserved(byte) {
            put(byte)?;
        } else {
            put(b'%')?;
            put(HEX[usize::from(byte >> 4)])?;
            put(HEX[usize::from(byte & 0x0f)])?;
        }
    }

    // Only ASCII was written.
    core::str::from_utf8(&out[..written]).map_err(|_| Error::BufferOverflow)
}

/// Base64-encode `input` (standard alphabet, with padding) into `out`.
///
/// # Errors
///
/// Returns [`Error::BufferOverflow`] if `out` is too small. The output is
/// `4 * input.len().div_ceil(3)` bytes long.
pub fn base64_encode<'a>(input: &[u8], out: &'a mut [u8]) -> Result<&'a str, Error> {
    let mut written = 0;
    let mut put = |byte: u8| -> Result<(), Error> {
        *out.get_mut(written).ok_or(Error::BufferOverflow)? = byte;
        written += 1;
        Ok(())
    };
    base64_write(input.iter().copied(), &mut put)?;
    core::str::from_utf8(&out[..written]).map_err(|_| Error::BufferOverflow)
}

/// Feed the Base64 encoding of `bytes` to `put`, one output byte at a time.
fn base64_write(
    mut bytes: impl Iterator<Item = u8>,
    put: &mut impl FnMut(u8) -> Result<(), Error>,
) -> Result<(), Error> {
    while let Some(first) = bytes.next() {
        let second = bytes.next();
        let third = if second.is_some() { bytes.next() } else { None };

        let group = u32::from(first) << 16
            | u32::from(second.unwrap_or(0)) << 8
            | u32::from(third.unwrap_or(0));
        put(BASE64[(group >> 18) as usize & 0x3f])?;
        put(BASE64[(group >> 12) as usize & 0x3f])?;
        put(if second.is_some() {
            BASE64[(group >> 6) as usize & 0x3f]
        } else {
            b'='
        })?;
        put(if third.is_some() {
            BASE64[group as usize & 0x3f]
        } else {
            b'='
        })?;
    }
    Ok(())
}

/// Build the value of a `Authorization: Basic ...` header (RFC 7617) from a
/// user name and password, for example `Basic dXNlcjpwYXNz`.
///
/// `N` must fit `"Basic "` plus `4 * (user.len() + 1 + password.len()).div_ceil(3)`
/// bytes.
///
/// # Errors
///
/// Returns [`Error::BufferOverflow`] if the value does not fit in `N` bytes,
/// or [`Error::HeaderError`] if `user` contains `:`, which cannot be
/// represented.
pub fn basic_auth<const N: usize>(user: &str, password: &str) -> Result<String<N>, Error> {
    if user.contains(':') {
        return Err(Error::HeaderError("Basic auth user name contains ':'"));
    }
    let mut value = String::new();
    value
        .push_str("Basic ")
        .map_err(|_| Error::BufferOverflow)?;
    base64_write(
        user.bytes()
            .chain(core::iter::once(b':'))
            .chain(password.bytes()),
        &mut |byte| {
            value
                .push(char::from(byte))
                .map_err(|_| Error::BufferOverflow)
        },
    )?;
    Ok(value)
}

/// Build the value of a `Authorization: Bearer ...` header (RFC 6750), for
/// example `Bearer abc123`.
///
/// # Errors
///
/// Returns [`Error::BufferOverflow`] if the value does not fit in `N` bytes.
pub fn bearer_auth<const N: usize>(token: &str) -> Result<String<N>, Error> {
    let mut value = String::new();
    write!(value, "Bearer {token}").map_err(|_| Error::BufferOverflow)?;
    Ok(value)
}

/// Builds a `name=value&name=value` string, percent-encoding every name and
/// value, for query strings and `application/x-www-form-urlencoded` bodies.
///
/// ```
/// use nanofish::encode::QueryBuilder;
///
/// let mut query = QueryBuilder::<64>::new();
/// query.pair("q", "a&b c").unwrap().pair("limit", "10").unwrap();
/// assert_eq!(query.as_str(), "q=a%26b%20c&limit=10");
/// ```
#[derive(Debug, Clone, Default)]
pub struct QueryBuilder<const N: usize> {
    buffer: String<N>,
}

impl<const N: usize> QueryBuilder<N> {
    /// Create an empty builder.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buffer: String::new(),
        }
    }

    /// Append `name=value`, separated from the previous pair by `&`.
    ///
    /// On error the builder is left unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`Error::BufferOverflow`] if the pair does not fit in `N` bytes.
    pub fn pair(&mut self, name: &str, value: &str) -> Result<&mut Self, Error> {
        let rollback = self.buffer.len();
        let result = self.push_pair(name, value);
        if result.is_err() {
            self.buffer.truncate(rollback);
        }
        result.map(|()| self)
    }

    fn push_pair(&mut self, name: &str, value: &str) -> Result<(), Error> {
        if !self.buffer.is_empty() {
            self.buffer.push('&').map_err(|_| Error::BufferOverflow)?;
        }
        self.push_encoded(name)?;
        self.buffer.push('=').map_err(|_| Error::BufferOverflow)?;
        self.push_encoded(value)
    }

    fn push_encoded(&mut self, component: &str) -> Result<(), Error> {
        for byte in component.bytes() {
            if is_unreserved(byte) {
                self.buffer
                    .push(char::from(byte))
                    .map_err(|_| Error::BufferOverflow)?;
            } else {
                self.buffer
                    .push('%')
                    .and_then(|()| self.buffer.push(char::from(HEX[usize::from(byte >> 4)])))
                    .and_then(|()| self.buffer.push(char::from(HEX[usize::from(byte & 0x0f)])))
                    .map_err(|_| Error::BufferOverflow)?;
            }
        }
        Ok(())
    }

    /// The encoded string so far, without a leading `?`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.buffer
    }

    /// The encoded string as bytes, for use as a form request body.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.buffer.as_bytes()
    }

    /// Whether no pair has been added.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use nanofish::encode::*;
    use nanofish::request::percent_decode;

    #[test]
    fn test_percent_encode() {
        let mut out = [0u8; 64];
        assert_eq!(percent_encode("", &mut out).unwrap(), "");
        assert_eq!(
            percent_encode("abc-._~XYZ09", &mut out).unwrap(),
            "abc-._~XYZ09"
        );
        assert_eq!(
            percent_encode("a b&c=d#e/f?g+h%", &mut out).unwrap(),
            "a%20b%26c%3Dd%23e%2Ff%3Fg%2Bh%25"
        );
        // Non-ASCII is encoded byte-wise as UTF-8.
        assert_eq!(percent_encode("é", &mut out).unwrap(), "%C3%A9");
        // A control character cannot reach the wire unescaped.
        assert_eq!(percent_encode("a\r\nb", &mut out).unwrap(), "a%0D%0Ab");
    }

    #[test]
    fn test_percent_encode_round_trips_with_decode() {
        let mut encoded = [0u8; 96];
        let mut decoded = [0u8; 32];
        for input in ["hello world", "a&b=c", "ünï ✓", "100%", "~-._"] {
            let encoded = percent_encode(input, &mut encoded).unwrap();
            assert_eq!(percent_decode(encoded, &mut decoded).unwrap(), input);
        }
    }

    #[test]
    fn test_percent_encode_overflow() {
        let mut out = [0u8; 4];
        assert_eq!(percent_encode("abcd", &mut out).unwrap(), "abcd");
        assert!(matches!(
            percent_encode("abcde", &mut out),
            Err(Error::BufferOverflow)
        ));
        // `%XX` needs three bytes.
        assert!(matches!(
            percent_encode("ab ", &mut out[..2]),
            Err(Error::BufferOverflow)
        ));
    }

    #[test]
    fn test_base64_encode_rfc4648_vectors() {
        let mut out = [0u8; 16];
        for (input, expected) in [
            (&b""[..], ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64_encode(input, &mut out).unwrap(), expected);
        }
        // Bytes outside ASCII use the whole alphabet, including `+` and `/`.
        assert_eq!(
            base64_encode(&[0xfb, 0xff, 0xbf], &mut out).unwrap(),
            "+/+/"
        );
    }

    #[test]
    fn test_base64_encode_overflow() {
        let mut out = [0u8; 7];
        assert!(matches!(
            base64_encode(b"foobar", &mut out),
            Err(Error::BufferOverflow)
        ));
        let mut out = [0u8; 8];
        assert_eq!(base64_encode(b"foobar", &mut out).unwrap(), "Zm9vYmFy");
    }

    #[test]
    fn test_basic_auth() {
        // RFC 7617 example.
        assert_eq!(
            basic_auth::<40>("Aladdin", "open sesame").unwrap().as_str(),
            "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );
        assert_eq!(
            basic_auth::<32>("user", "").unwrap().as_str(),
            "Basic dXNlcjo="
        );
        assert!(matches!(
            basic_auth::<8>("user", "pass"),
            Err(Error::BufferOverflow)
        ));
        assert!(matches!(
            basic_auth::<64>("a:b", "pass"),
            Err(Error::HeaderError(_))
        ));
        // A colon in the password is fine.
        assert!(basic_auth::<64>("user", "pa:ss").is_ok());
    }

    #[test]
    fn test_bearer_auth() {
        assert_eq!(
            bearer_auth::<32>("abc.def").unwrap().as_str(),
            "Bearer abc.def"
        );
        assert!(matches!(
            bearer_auth::<8>("abc.def"),
            Err(Error::BufferOverflow)
        ));
    }

    #[test]
    fn test_query_builder() {
        let mut query = QueryBuilder::<64>::new();
        assert!(query.is_empty());
        query
            .pair("q", "a&b c")
            .unwrap()
            .pair("limit", "10")
            .unwrap()
            .pair("empty", "")
            .unwrap();
        assert_eq!(query.as_str(), "q=a%26b%20c&limit=10&empty=");
        assert_eq!(query.as_bytes(), b"q=a%26b%20c&limit=10&empty=");
        assert!(!query.is_empty());
    }

    #[test]
    fn test_query_builder_overflow_leaves_builder_unchanged() {
        let mut query = QueryBuilder::<12>::new();
        query.pair("a", "1").unwrap();
        assert!(matches!(
            query.pair("longer", "value"),
            Err(Error::BufferOverflow)
        ));
        assert_eq!(query.as_str(), "a=1");
        // Still usable afterwards.
        query.pair("b", "2").unwrap();
        assert_eq!(query.as_str(), "a=1&b=2");

        // Overflow in the middle of a `%XX` escape.
        let mut query = QueryBuilder::<4>::new();
        assert!(matches!(query.pair("a", " "), Err(Error::BufferOverflow)));
        assert!(query.is_empty());
    }
}
