//! HTTP client ports and adapters.

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
pub use tls::{DefaultHttpTlsClient, HttpTlsClient, SmallHttpTlsClient};
