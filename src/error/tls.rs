//! Conversions from `embedded-tls` errors, compiled with the `tls` feature.

use nanofish::error::Error;

impl From<embedded_tls::TlsError> for Error {
    fn from(err: embedded_tls::TlsError) -> Self {
        Self::TlsError(err)
    }
}
