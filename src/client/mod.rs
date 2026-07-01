//! HTTP client ports and adapters.

#[cfg(feature = "embassy")]
mod embassy;
mod io;

#[cfg(feature = "embassy")]
pub use embassy::{DefaultEmbassyHttpClient, EmbassyHttpClient, SmallEmbassyHttpClient};
pub use io::{DefaultHttpClient, HttpClient, HttpClientRequest, SmallHttpClient};
#[cfg(feature = "tls")]
pub use io::{DefaultHttpTlsClient, HttpTlsClient, SmallHttpTlsClient};
