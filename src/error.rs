/// Errors that can occur during HTTP operations
///
/// This enum represents all possible errors that can be returned by the HTTP client
/// during various stages of request processing, from URL parsing to connection
/// establishment and response handling.
#[derive(Debug)]
pub enum Error {
    /// The provided URL was invalid or malformed
    InvalidUrl,
    /// DNS resolution failed
    DnsError(embassy_net::dns::Error),
    /// No IP addresses were returned by DNS resolution
    IpAddressEmpty,
    /// Failed to establish a TCP connection
    ConnectionError(embassy_net::tcp::ConnectError),
    /// TCP communication error
    TcpError(embassy_net::tcp::Error),
    /// No response was received from the server
    NoResponse,
    /// The server's response could not be parsed
    InvalidResponse(&'static str),
    /// This error occurs when there is an issue with the TLS handshake or communication.
    #[cfg(feature = "tls")]
    TlsError(embedded_tls::TlsError),
    // Scheme not supported
    UnsupportedScheme(&'static str),
}

impl defmt::Format for Error {
    fn format(&self, fmt: defmt::Formatter) {
        defmt::write!(fmt, "{:?}", self)
    }
}

impl From<embassy_net::dns::Error> for Error {
    fn from(err: embassy_net::dns::Error) -> Self {
        Error::DnsError(err)
    }
}

impl From<embassy_net::tcp::ConnectError> for Error {
    fn from(err: embassy_net::tcp::ConnectError) -> Self {
        Error::ConnectionError(err)
    }
}

impl From<embassy_net::tcp::Error> for Error {
    fn from(err: embassy_net::tcp::Error) -> Self {
        Error::TcpError(err)
    }
}

#[cfg(feature = "tls")]
impl From<embedded_tls::TlsError> for Error {
    fn from(err: embedded_tls::TlsError) -> Self {
        Error::TlsError(err)
    }
}
