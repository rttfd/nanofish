use core::future::Future;

use crate::client::io::{HttpClient, HttpClientRequest};
use crate::client::{DEFAULT_REQUEST_SIZE, SMALL_REQUEST_SIZE};
use crate::{error::Error, options::HttpClientOptions, response::HttpResponse};
use embedded_io_async::{Read, Write};
use embedded_tls::Aes128GcmSha256;

const DEFAULT_TLS_BUFFER_SIZE: usize = 4096;
const SMALL_TLS_BUFFER_SIZE: usize = 1024;

/// How an [`HttpTlsClient`] authenticates the server.
///
/// The policy is stored in the client; verified and unverified clients use
/// the same request API.
pub enum TlsVerification<'a, RNG> {
    /// Do not verify the server certificate: the connection is vulnerable to
    /// man-in-the-middle attacks. Only use this on trusted networks, or when
    /// TLS is terminated by something else (for example a reverse proxy).
    Unverified(RNG),
    /// Verify the server certificate with a caller-built verifier, for
    /// example [`embedded_tls::pki::CertVerifier`] with a pinned root CA
    /// (DER). This checks the chain of trust, the hostname against the
    /// request's `server_name`, and — when the verifier's clock yields Unix
    /// time — the certificate validity period.
    ///
    /// The RNG must be a cryptographic random number generator; it seeds the
    /// TLS 1.3 key exchange.
    Verified(RNG, &'a mut dyn TlsVerifier<Aes128GcmSha256>),
}

/// TLS server certificate verifier for [`TlsVerification::Verified`],
/// re-exported for convenience.
pub use embedded_tls::TlsVerifier;

/// Transport-generic HTTPS client for already-connected streams.
///
/// The client holds an [`HttpClientOptions`] and a [`TlsVerification`]
/// policy; each [`request`](Self::request) call supplies the
/// already-connected `embedded-io-async` stream, the `server_name`, the
/// request, and the response buffer.
///
/// # Security
///
/// With [`TlsVerification::Unverified`] the server certificate is **not**
/// checked: the connection is vulnerable to man-in-the-middle attacks.
/// Prefer [`TlsVerification::Verified`] with a root CA wherever possible.
/// See the [`TlsVerification`] variants for the exact guarantees.
pub struct HttpTlsClient<
    'a,
    RNG,
    const RQ: usize = DEFAULT_REQUEST_SIZE,
    const TLS_READ: usize = DEFAULT_TLS_BUFFER_SIZE,
    const TLS_WRITE: usize = DEFAULT_TLS_BUFFER_SIZE,
> {
    options: HttpClientOptions,
    verification: TlsVerification<'a, RNG>,
}

/// Type alias for [`HttpTlsClient`] with default request and TLS buffer sizes.
pub type DefaultHttpTlsClient<'a, RNG> =
    HttpTlsClient<'a, RNG, DEFAULT_REQUEST_SIZE, DEFAULT_TLS_BUFFER_SIZE, DEFAULT_TLS_BUFFER_SIZE>;

/// Type alias for [`HttpTlsClient`] with smaller request and TLS buffer sizes.
pub type SmallHttpTlsClient<'a, RNG> =
    HttpTlsClient<'a, RNG, SMALL_REQUEST_SIZE, SMALL_TLS_BUFFER_SIZE, SMALL_TLS_BUFFER_SIZE>;

impl<'a, RNG, const RQ: usize, const TLS_READ: usize, const TLS_WRITE: usize>
    HttpTlsClient<'a, RNG, RQ, TLS_READ, TLS_WRITE>
{
    /// Create a client that does **not** verify the server certificate.
    ///
    /// See [`TlsVerification::Unverified`] for the security implications.
    #[must_use]
    pub fn unverified(rng: RNG) -> Self {
        Self::with_verification(TlsVerification::Unverified(rng))
    }

    /// Create a client that verifies the server certificate with a
    /// caller-built verifier, for example [`embedded_tls::pki::CertVerifier`]
    /// with a pinned root CA.
    ///
    /// See [`TlsVerification::Verified`] for the guarantees.
    #[must_use]
    pub fn verified(rng: RNG, verifier: &'a mut dyn TlsVerifier<Aes128GcmSha256>) -> Self {
        Self::with_verification(TlsVerification::Verified(rng, verifier))
    }

    /// Create a client with a custom [`TlsVerification`] policy and default
    /// options.
    #[must_use]
    pub fn with_verification(verification: TlsVerification<'a, RNG>) -> Self {
        Self {
            options: HttpClientOptions::default(),
            verification,
        }
    }

    /// Create a client with custom options and a [`TlsVerification`] policy.
    #[must_use]
    pub const fn with_options(
        options: HttpClientOptions,
        verification: TlsVerification<'a, RNG>,
    ) -> Self {
        Self {
            options,
            verification,
        }
    }

    /// The configured verification policy.
    #[must_use]
    pub const fn verification(&self) -> &TlsVerification<'a, RNG> {
        &self.verification
    }

    /// Send one HTTPS request over an already-connected stream.
    ///
    /// The client's [`TlsVerification`] policy selects whether the handshake
    /// verifies the server certificate. Read errors are retried up to
    /// [`HttpClientOptions::max_retries`] times without pausing between
    /// attempts; use [`HttpTlsClient::request_with_retry_delay`] to sleep
    /// between retries.
    ///
    /// # Errors
    ///
    /// Returns an error if the TLS handshake fails, request construction
    /// fails, stream IO fails, no response is received, the response is
    /// incomplete or larger than the response buffer, or the response cannot
    /// be parsed.
    #[expect(clippy::future_not_send)]
    pub async fn request<'b, S>(
        &mut self,
        stream: S,
        server_name: &str,
        request: HttpClientRequest<'_>,
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error>
    where
        S: Read + Write,
        RNG: embedded_tls::CryptoRngCore,
    {
        self.request_with_retry_delay(stream, server_name, request, response_buffer, || {
            core::future::ready(())
        })
        .await
    }

    /// Send one HTTPS request over an already-connected stream, sleeping via
    /// `retry_delay` between read retries.
    ///
    /// Same signature as [`HttpTlsClient::request`]; verified and unverified
    /// clients share this code path.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpTlsClient::request`].
    #[expect(clippy::future_not_send)]
    pub async fn request_with_retry_delay<'b, S, D, Fut>(
        &mut self,
        stream: S,
        server_name: &str,
        request: HttpClientRequest<'_>,
        response_buffer: &'b mut [u8],
        retry_delay: D,
    ) -> Result<(HttpResponse<'b>, usize), Error>
    where
        S: Read + Write,
        RNG: embedded_tls::CryptoRngCore,
        D: Fn() -> Fut,
        Fut: Future<Output = ()>,
    {
        let tls_config = embedded_tls::TlsConfig::new().with_server_name(server_name);
        let mut read_record_buffer = [0; TLS_READ];
        let mut write_record_buffer = [0; TLS_WRITE];
        let mut tls = embedded_tls::TlsConnection::new(
            stream,
            &mut read_record_buffer,
            &mut write_record_buffer,
        );

        match &mut self.verification {
            TlsVerification::Unverified(rng) => {
                tls.open(embedded_tls::TlsContext::new(
                    &tls_config,
                    embedded_tls::UnsecureProvider::new::<Aes128GcmSha256>(rng),
                ))
                .await?;
            }
            TlsVerification::Verified(rng, verifier) => {
                tls.open(embedded_tls::TlsContext::new(
                    &tls_config,
                    VerifiedTlsProvider {
                        rng,
                        verifier: RefVerifier(&mut **verifier),
                    },
                ))
                .await?;
            }
        }

        let client = HttpClient::<RQ>::with_options(self.options);
        let result = client
            .request_with_retry_delay(&mut tls, request, response_buffer, retry_delay)
            .await;
        let _ = tls.close().await;
        result
    }
}

/// Crypto provider used for [`TlsVerification::Verified`]: defers certificate
/// verification to the caller-supplied [`TlsVerifier`].
/// Sized forwarder so the provider can hand the handshake a `&mut V` even
/// when `V` is a `dyn TlsVerifier`.
struct RefVerifier<'p, V: ?Sized>(&'p mut V);

impl<V> TlsVerifier<Aes128GcmSha256> for RefVerifier<'_, V>
where
    V: TlsVerifier<Aes128GcmSha256> + ?Sized,
{
    fn set_hostname_verification(&mut self, hostname: &str) -> Result<(), embedded_tls::TlsError> {
        self.0.set_hostname_verification(hostname)
    }

    fn verify_certificate(
        &mut self,
        transcript: &<Aes128GcmSha256 as embedded_tls::TlsCipherSuite>::Hash,
        cert: embedded_tls::CertificateRef<'_>,
    ) -> Result<(), embedded_tls::TlsError> {
        self.0.verify_certificate(transcript, cert)
    }

    fn verify_signature(
        &mut self,
        verify: embedded_tls::CertificateVerifyRef<'_>,
    ) -> Result<(), embedded_tls::TlsError> {
        self.0.verify_signature(verify)
    }
}

/// Crypto provider used for [`TlsVerification::Verified`]: defers certificate
/// verification to the caller-supplied [`TlsVerifier`].
struct VerifiedTlsProvider<'p, RNG, V>
where
    V: TlsVerifier<Aes128GcmSha256> + ?Sized,
{
    rng: &'p mut RNG,
    verifier: RefVerifier<'p, V>,
}

impl<RNG, V> embedded_tls::CryptoProvider for VerifiedTlsProvider<'_, RNG, V>
where
    RNG: embedded_tls::CryptoRngCore,
    V: TlsVerifier<Aes128GcmSha256> + ?Sized,
{
    type CipherSuite = Aes128GcmSha256;

    /// Client-certificate authentication is not supported; `signer()` keeps
    /// its default (failing) implementation and this type is never used.
    type Signature = [u8; 0];

    fn rng(&mut self) -> impl embedded_tls::CryptoRngCore {
        &mut *self.rng
    }

    fn verifier(
        &mut self,
    ) -> Result<&mut impl TlsVerifier<Self::CipherSuite>, embedded_tls::TlsError> {
        Ok(&mut self.verifier)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic LCG RNG for wiring tests; not for production use.
    struct TestRng(u32);

    impl rand_core::RngCore for TestRng {
        fn next_u32(&mut self) -> u32 {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            self.0
        }

        fn next_u64(&mut self) -> u64 {
            (u64::from(self.next_u32()) << 32) | u64::from(self.next_u32())
        }

        fn fill_bytes(&mut self, dest: &mut [u8]) {
            for chunk in dest.chunks_mut(4) {
                let value = self.next_u32().to_le_bytes();
                chunk.copy_from_slice(&value[..chunk.len()]);
            }
        }

        fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
            self.fill_bytes(dest);
            Ok(())
        }
    }

    impl rand_core::CryptoRng for TestRng {}

    #[test]
    fn test_unverified_client_construction() {
        // The unverified policy needs no TLS types at the call site.
        let client: DefaultHttpTlsClient<TestRng> = HttpTlsClient::unverified(TestRng(1));
        assert!(matches!(
            client.verification(),
            TlsVerification::Unverified(_)
        ));
    }

    #[test]
    fn test_verified_client_construction() {
        // Caller pins a root CA (contents irrelevant for the wiring test) and
        // picks the clock; hostname checks flow from the request's server name.
        let ca_der = [0u8; 32];
        let mut verifier =
            embedded_tls::pki::CertVerifier::<Aes128GcmSha256, embedded_tls::NoClock, 4096>::new(
                embedded_tls::Certificate::X509(&ca_der),
            );
        verifier.set_hostname_verification("example.com").unwrap();

        let client: DefaultHttpTlsClient<TestRng> =
            HttpTlsClient::verified(TestRng(2), &mut verifier);
        assert!(matches!(
            client.verification(),
            TlsVerification::Verified(_, _)
        ));
    }
}
