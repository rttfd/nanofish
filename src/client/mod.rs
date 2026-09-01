//! HTTP client ports and adapters.

/// Default request buffer size shared by the transport-generic clients.
pub(crate) const DEFAULT_REQUEST_SIZE: usize = 1024;
/// Small request buffer size shared by the transport-generic clients.
pub(crate) const SMALL_REQUEST_SIZE: usize = 512;

#[cfg(feature = "embassy")]
mod embassy;
mod io;
#[cfg(feature = "tls")]
mod tls;

#[cfg(feature = "embassy")]
pub use embassy::{DefaultEmbassyHttpClient, EmbassyHttpClient, SmallEmbassyHttpClient};
pub use io::{
    DefaultHttpClient, HttpClient, HttpClientRequest, HttpEndpoint, SmallHttpClient, parse_endpoint,
};
#[cfg(feature = "tls")]
pub use tls::{DefaultHttpTlsClient, HttpTlsClient, SmallHttpTlsClient, TlsVerification};
