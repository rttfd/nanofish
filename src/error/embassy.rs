//! Conversions from `embassy-net` errors, compiled with the `embassy` feature.

use nanofish::error::Error;

impl From<embassy_net::dns::Error> for Error {
    fn from(_err: embassy_net::dns::Error) -> Self {
        Self::DnsError
    }
}

impl From<embassy_net::tcp::ConnectError> for Error {
    fn from(_err: embassy_net::tcp::ConnectError) -> Self {
        Self::ConnectionError
    }
}

impl From<embassy_net::tcp::Error> for Error {
    fn from(err: embassy_net::tcp::Error) -> Self {
        Self::io(err)
    }
}

#[cfg(test)]
mod tests {
    use embassy_net::{dns, tcp};
    use nanofish::error::Error;

    #[test]
    fn test_from_dns_error() {
        let dns_err = dns::Error::InvalidName;
        let err: Error = dns_err.into();
        match err {
            Error::DnsError => {}
            _ => panic!("Expected DnsError variant"),
        }
    }

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
