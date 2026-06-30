//! HTTP client ports and adapters.

#[cfg(feature = "embassy")]
mod embassy;
mod io;

#[cfg(feature = "embassy")]
pub use embassy::{DefaultHttpClient, HttpClient, SmallHttpClient};
pub use io::{DefaultHttpIoClient, HttpIoClient, HttpIoRequest, SmallHttpIoClient};
#[cfg(feature = "tls")]
pub use io::{DefaultHttpTlsIoClient, HttpTlsIoClient, SmallHttpTlsIoClient};
