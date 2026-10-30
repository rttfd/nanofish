/// Errors that can occur during HTTP operations
///
/// This enum represents all possible errors that can be returned by the HTTP client
/// during various stages of request processing, from URL parsing to connection
/// establishment and response handling.
#[derive(Debug)]
/// All possible errors returned by the HTTP client.
pub enum Error {
    /// The provided URL was invalid or malformed
    InvalidUrl,
    /// DNS resolution failed
    DnsError,
    /// No IP addresses were returned by DNS resolution
    IpAddressEmpty,
    /// Failed to establish a TCP connection
    ConnectionError,
    /// TCP communication error, with the kind reported by the transport
    TcpError(embedded_io_async::ErrorKind),
    /// No response was received from the server
    NoResponse,
    /// The server's response could not be parsed
    InvalidResponse(&'static str),
    /// This error occurs when there is an issue with the TLS handshake or communication.
    #[cfg(feature = "tls")]
    TlsError(embedded_tls::TlsError),
    /// Scheme not supported
    UnsupportedScheme(&'static str),
    /// Header error, e.g. too long name or value
    HeaderError(&'static str),
    /// Invalid status code received from the server
    InvalidStatusCode,
    /// Buffer overflow when building a request or response
    BufferOverflow,
    /// The operation did not finish within its overall deadline
    Timeout,
}

#[cfg(feature = "defmt")]
impl defmt::Format for Error {
    fn format(&self, fmt: defmt::Formatter) {
        defmt::write!(fmt, "{:?}", defmt::Debug2Format(self));
    }
}

#[cfg(feature = "embassy")]
impl From<embassy_net::dns::Error> for Error {
    fn from(_err: embassy_net::dns::Error) -> Self {
        Self::DnsError
    }
}

#[cfg(feature = "embassy")]
impl From<embassy_net::tcp::ConnectError> for Error {
    fn from(_err: embassy_net::tcp::ConnectError) -> Self {
        Self::ConnectionError
    }
}

#[cfg(feature = "embassy")]
impl From<embassy_net::tcp::Error> for Error {
    fn from(err: embassy_net::tcp::Error) -> Self {
        Self::io(err)
    }
}

#[cfg(feature = "tls")]
impl From<embedded_tls::TlsError> for Error {
    fn from(err: embedded_tls::TlsError) -> Self {
        Self::TlsError(err)
    }
}

impl Error {
    /// Wrap a transport I/O error, keeping its [`embedded_io_async::ErrorKind`].
    #[expect(
        clippy::needless_pass_by_value,
        reason = "taken by value so it can be passed straight to `map_err`"
    )]
    pub(crate) fn io<E: embedded_io_async::Error>(error: E) -> Self {
        Self::TcpError(error.kind())
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidUrl => write!(f, "Invalid URL"),
            Self::DnsError => write!(f, "DNS resolution failed"),
            Self::IpAddressEmpty => write!(f, "No IP addresses returned by DNS"),
            Self::ConnectionError => write!(f, "Failed to establish TCP connection"),
            Self::TcpError(kind) => write!(f, "TCP communication error: {kind:?}"),
            Self::NoResponse => write!(f, "No response received from server"),
            Self::InvalidResponse(msg) => write!(f, "Invalid response: {msg}"),
            #[cfg(feature = "tls")]
            Self::TlsError(_) => write!(f, "TLS error occurred"),
            Self::UnsupportedScheme(scheme) => write!(f, "Unsupported scheme: {scheme}"),
            Self::HeaderError(msg) => write!(f, "Header error: {msg}"),
            Self::InvalidStatusCode => write!(f, "Invalid status code"),
            Self::BufferOverflow => write!(f, "Buffer overflow"),
            Self::Timeout => write!(f, "Operation timed out"),
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "embassy")]
    use embassy_net::{dns, tcp};
    use nanofish::error::*;

    #[test]
    fn test_error_display() {
        let e = Error::InvalidUrl;
        assert_eq!(format!("{e}"), "Invalid URL");
        let e = Error::IpAddressEmpty;
        assert_eq!(format!("{e}"), "No IP addresses returned by DNS");
        let e = Error::NoResponse;
        assert_eq!(format!("{e}"), "No response received from server");
        let e = Error::InvalidResponse("bad");
        assert_eq!(format!("{e}"), "Invalid response: bad");
        let e = Error::UnsupportedScheme("ftp");
        assert_eq!(format!("{e}"), "Unsupported scheme: ftp");
        let e = Error::HeaderError("too long");
        assert_eq!(format!("{e}"), "Header error: too long");
        let e = Error::InvalidStatusCode;
        assert_eq!(format!("{e}"), "Invalid status code");
        let e = Error::Timeout;
        assert_eq!(format!("{e}"), "Operation timed out");
        let e = Error::TcpError(embedded_io_async::ErrorKind::TimedOut);
        assert_eq!(format!("{e}"), "TCP communication error: TimedOut");
    }

    #[cfg(feature = "embassy")]
    #[test]
    fn test_from_dns_error() {
        let dns_err = dns::Error::InvalidName;
        let err: Error = dns_err.into();
        match err {
            Error::DnsError => {}
            _ => panic!("Expected DnsError variant"),
        }
    }

    #[cfg(feature = "embassy")]
    #[test]
    fn test_from_tcp_error() {
        let tcp_err = tcp::Error::ConnectionReset;
        let err: Error = tcp_err.into();
        match err {
            Error::TcpError(embedded_io_async::ErrorKind::ConnectionReset) => {}
            _ => panic!("Expected TcpError(ConnectionReset) variant"),
        }
    }
}
