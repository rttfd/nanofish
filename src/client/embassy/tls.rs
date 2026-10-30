//! HTTPS for the Embassy client, compiled with the `tls` feature.

use embassy_net::tcp::TcpSocket;
use embassy_time::{Instant, Timer, with_timeout};
use embedded_tls::{Aes128GcmSha256, TlsConfig, TlsConnection, TlsContext, UnsecureProvider};
use nanofish::client::embassy::{EmbassyHttpClient, HOST_HEADER_SIZE, resolve_host};
use nanofish::client::io::{
    HttpClient as BaseHttpClient, HttpClientRequest, HttpEndpoint, parse_endpoint,
};
use nanofish::client::stream::{BodySink, StreamedResponse};
use nanofish::client::tls::HttpTlsClient;
use nanofish::{error::Error, header::HttpHeader, method::HttpMethod, response::HttpResponse};
use rand_core::{CryptoRng, RngCore};

impl<
    const TCP_RX: usize,
    const TCP_TX: usize,
    const TLS_READ: usize,
    const TLS_WRITE: usize,
    const RQ: usize,
> EmbassyHttpClient<'_, TCP_RX, TCP_TX, TLS_READ, TLS_WRITE, RQ>
{
    /// Connect to `endpoint` over TLS, run `operation` on the connection, and
    /// close it.
    #[expect(clippy::future_not_send)]
    async fn with_https_connection<R>(
        &self,
        endpoint: &HttpEndpoint<'_>,
        operation: impl AsyncFnOnce(
            &mut TlsConnection<'_, TcpSocket<'_>, Aes128GcmSha256>,
        ) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let (host, port) = (endpoint.host, endpoint.port);
        let mut rx_buffer = [0; TCP_RX];
        let mut tx_buffer = [0; TCP_TX];
        let mut socket = TcpSocket::new(*self.stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(embassy_time::Duration::from_millis(
            self.options.socket_timeout.as_millis(),
        )));

        let ip_addr = resolve_host(*self.stack, host).await?;
        let remote_endpoint = (ip_addr, port);

        socket
            .connect(remote_endpoint)
            .await
            .map_err(|e: embassy_net::tcp::ConnectError| {
                socket.abort();
                Error::from(e)
            })?;

        let mut read_record_buffer = [0; TLS_READ];
        let mut write_record_buffer = [0; TLS_WRITE];

        let tls_config = TlsConfig::new().with_server_name(host);
        let mut tls = TlsConnection::new(socket, &mut read_record_buffer, &mut write_record_buffer);
        let rng = XorShift32Rng::new(tls_seed(Instant::now().as_ticks()));

        tls.open(TlsContext::new(
            &tls_config,
            UnsecureProvider::new::<Aes128GcmSha256>(rng),
        ))
        .await?;

        let result = operation(&mut tls).await;

        if let Err((_, e)) = tls.close().await {
            debug!("Error closing TLS connection: {:?}", Error::from(e));
        }

        Timer::after(embassy_time::Duration::from_millis(
            self.options.socket_close_delay.as_millis(),
        ))
        .await;

        result
    }

    /// Make HTTPS request over TLS with zero-copy response handling
    #[expect(clippy::future_not_send)]
    pub(super) async fn make_https_request<'b>(
        &self,
        method: HttpMethod,
        endpoint: &HttpEndpoint<'_>,
        headers: &[HttpHeader<'_>],
        body: Option<&[u8]>,
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error> {
        let host_header = endpoint.host_header::<HOST_HEADER_SIZE>()?;
        let client = BaseHttpClient::<RQ>::with_options(self.options);
        self.with_https_connection(endpoint, async |tls| {
            client
                .request_with_retry_delay(
                    tls,
                    HttpClientRequest {
                        method,
                        host: &host_header,
                        path: endpoint.path,
                        query: endpoint.query,
                        headers,
                        body,
                    },
                    response_buffer,
                    || self.retry_delay(),
                )
                .await
        })
        .await
    }

    /// Stream the response to an `https` request; see
    /// [`EmbassyHttpClient::request_streaming`].
    #[expect(clippy::future_not_send)]
    pub(super) async fn https_streaming<K: BodySink>(
        &self,
        endpoint: &HttpEndpoint<'_>,
        client: &BaseHttpClient<RQ>,
        request: HttpClientRequest<'_>,
        buffer: &mut [u8],
        sink: &mut K,
    ) -> Result<StreamedResponse, Error> {
        self.with_https_connection(endpoint, async |tls| {
            client
                .request_streaming_with_retry_delay(tls, request, buffer, sink, || {
                    self.retry_delay()
                })
                .await
        })
        .await
    }

    /// Make an HTTPS request with a caller-supplied [`HttpTlsClient`], which
    /// decides how the server certificate is verified.
    ///
    /// [`EmbassyHttpClient::request`] does not verify certificates. Use this
    /// method with [`HttpTlsClient::verified`] to check the server against a
    /// pinned root CA (or any other [`TlsVerifier`](nanofish::client::TlsVerifier)),
    /// while still using Embassy for DNS and TCP. The TLS client's own options
    /// apply to reads and retries; this client's [`HttpClientOptions::socket_timeout`](nanofish::HttpClientOptions::socket_timeout),
    /// [`HttpClientOptions::socket_close_delay`](nanofish::HttpClientOptions::socket_close_delay) and
    /// [`HttpClientOptions::total_timeout`](nanofish::HttpClientOptions::total_timeout) apply to the connection. Redirects
    /// are not followed.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnsupportedScheme`] unless `endpoint` is an `https`
    /// URL, otherwise the errors of [`EmbassyHttpClient::request`] and of
    /// [`HttpTlsClient::request`].
    #[expect(clippy::future_not_send)]
    pub async fn request_with_tls<
        'b,
        RNG,
        const TLS_RQ: usize,
        const TLS_RECORD_READ: usize,
        const TLS_RECORD_WRITE: usize,
    >(
        &self,
        tls: &mut HttpTlsClient<'_, RNG, TLS_RQ, TLS_RECORD_READ, TLS_RECORD_WRITE>,
        method: HttpMethod,
        endpoint: &str,
        headers: &[HttpHeader<'_>],
        body: Option<&[u8]>,
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error>
    where
        RNG: embedded_tls::CryptoRngCore,
    {
        let attempt = async {
            let endpoint = parse_endpoint(endpoint)?;
            if endpoint.scheme != "https" {
                return Err(Error::UnsupportedScheme("expected an https URL"));
            }
            let host_header = endpoint.host_header::<HOST_HEADER_SIZE>()?;
            self.with_http_connection(&endpoint, async |socket| {
                tls.request_with_retry_delay(
                    socket,
                    endpoint.host,
                    HttpClientRequest {
                        method,
                        host: &host_header,
                        path: endpoint.path,
                        query: endpoint.query,
                        headers,
                        body,
                    },
                    response_buffer,
                    || self.retry_delay(),
                )
                .await
            })
            .await
        };

        match self.options.total_timeout {
            Some(limit) => with_timeout(
                embassy_time::Duration::from_millis(limit.as_millis()),
                attempt,
            )
            .await
            .unwrap_or(Err(Error::Timeout)),
            None => attempt.await,
        }
    }
}

fn tls_seed(ticks: u64) -> u32 {
    /// Fallback seed for the one tick value that is absorbing for `XORShift32`.
    const ZERO_STATE_FALLBACK: u32 = 0x9E37_79B9;

    match (ticks & 0xFFFF_FFFF).try_into() {
        Ok(0) | Err(_) => ZERO_STATE_FALLBACK,
        Ok(seed) => seed,
    }
}

/// Simple `XORShift32` PRNG for TLS seeding.
/// Good enough for embedded TLS where the seed itself is time-based.
struct XorShift32Rng(u32);

impl XorShift32Rng {
    const fn new(seed: u32) -> Self {
        Self(seed)
    }
}

impl RngCore for XorShift32Rng {
    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    fn next_u64(&mut self) -> u64 {
        (u64::from(self.next_u32()) << 32) | u64::from(self.next_u32())
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        for chunk in dest.chunks_mut(4) {
            let val = self.next_u32();
            chunk.copy_from_slice(&val.to_le_bytes()[..chunk.len()]);
        }
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

impl CryptoRng for XorShift32Rng {}

#[cfg(test)]
mod tests {
    use nanofish::client::embassy::tls::*;

    #[test]
    fn test_tls_seed_avoids_zero_state() {
        // At tick zero (and anywhere the low 32 bits are zero) the seed must
        // fall back to a nonzero value: zero is absorbing for XORShift32.
        assert_eq!(tls_seed(0), 0x9E37_79B9);
        assert_eq!(tls_seed(0x1_0000_0000), 0x9E37_79B9);
        // The low 32 bits are used, so fresh tick values map to themselves.
        assert_eq!(tls_seed(1), 1);
        assert_eq!(tls_seed(0xDEAD_BEEF), 0xDEAD_BEEF);
        assert_eq!(tls_seed(0xFFFF_FFFF), 0xFFFF_FFFF);
    }
}
