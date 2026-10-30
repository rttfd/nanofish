//! HTTP client ports and adapters.

/// Default request buffer size shared by the transport-generic clients.
pub(crate) const DEFAULT_REQUEST_SIZE: usize = 1024;
/// Small request buffer size shared by the transport-generic clients.
pub(crate) const SMALL_REQUEST_SIZE: usize = 512;

mod builder;
#[cfg(feature = "embassy")]
mod embassy;
mod io;
#[cfg(any(feature = "embassy", test))]
pub(crate) mod redirect;
mod stream;
#[cfg(feature = "tls")]
mod tls;

pub use builder::RequestBuilder;
#[cfg(feature = "embassy")]
pub use embassy::{
    DefaultEmbassyHttpClient, EmbassyHttpClient, EmbassyHttpSession, SmallEmbassyHttpClient,
};
pub use io::{
    DefaultHttpClient, Exchange, HttpClient, HttpClientRequest, HttpEndpoint, SmallHttpClient,
    parse_endpoint,
};
pub use stream::{BodySink, StreamedResponse};
#[cfg(feature = "tls")]
pub use tls::{DefaultHttpTlsClient, HttpTlsClient, SmallHttpTlsClient, TlsVerification};
