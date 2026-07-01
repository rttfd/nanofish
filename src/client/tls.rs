use super::io::{HttpClient, HttpClientRequest};
use crate::{error::Error, options::HttpClientOptions, response::HttpResponse};
use embedded_io_async::{Read, Write};

const DEFAULT_REQUEST_SIZE: usize = 1024;
const SMALL_REQUEST_SIZE: usize = 1024;

/// Transport-generic HTTPS client for already-connected streams.
pub struct HttpTlsClient<
    const RQ: usize = DEFAULT_REQUEST_SIZE,
    const TLS_READ: usize = 4096,
    const TLS_WRITE: usize = 4096,
> {
    options: HttpClientOptions,
}

/// Type alias for `HttpTlsClient` with default request and TLS buffer sizes.
pub type DefaultHttpTlsClient = HttpTlsClient<DEFAULT_REQUEST_SIZE, 4096, 4096>;

/// Type alias for `HttpTlsClient` with smaller request and TLS buffer sizes.
pub type SmallHttpTlsClient = HttpTlsClient<SMALL_REQUEST_SIZE, 1024, 1024>;

impl Default for HttpTlsClient<DEFAULT_REQUEST_SIZE, 4096, 4096> {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpTlsClient<DEFAULT_REQUEST_SIZE, 4096, 4096> {
    /// Create a new transport-generic TLS client with default buffer sizes.
    #[must_use]
    pub fn new() -> Self {
        Self {
            options: HttpClientOptions::default(),
        }
    }
}

impl<const RQ: usize, const TLS_READ: usize, const TLS_WRITE: usize>
    HttpTlsClient<RQ, TLS_READ, TLS_WRITE>
{
    /// Create a new transport-generic TLS client with custom options.
    #[must_use]
    pub const fn with_options(options: HttpClientOptions) -> Self {
        Self { options }
    }

    /// Send one HTTPS request over an already-connected stream.
    ///
    /// # Errors
    ///
    /// Returns an error if TLS handshake/IO fails, request construction fails,
    /// no response is received, or the response cannot be parsed.
    #[expect(clippy::future_not_send)]
    pub async fn request<'b, S, RNG>(
        &self,
        stream: S,
        server_name: &str,
        request: HttpClientRequest<'_>,
        response_buffer: &'b mut [u8],
        rng: RNG,
    ) -> Result<(HttpResponse<'b>, usize), Error>
    where
        S: Read + Write,
        RNG: embedded_tls::CryptoRngCore,
    {
        let tls_config = embedded_tls::TlsConfig::new().with_server_name(server_name);
        let mut read_record_buffer = [0; TLS_READ];
        let mut write_record_buffer = [0; TLS_WRITE];
        let mut tls = embedded_tls::TlsConnection::new(
            stream,
            &mut read_record_buffer,
            &mut write_record_buffer,
        );

        tls.open(embedded_tls::TlsContext::new(
            &tls_config,
            embedded_tls::UnsecureProvider::new::<embedded_tls::Aes128GcmSha256>(rng),
        ))
        .await?;

        let client = HttpClient::<RQ>::with_options(self.options);
        let result = client.request(&mut tls, request, response_buffer).await;
        let _ = tls.close().await;
        result
    }
}
