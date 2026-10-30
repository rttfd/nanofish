//! Without the `tls` feature the Embassy client refuses `https` URLs.

use nanofish::client::embassy::EmbassyHttpClient;
use nanofish::client::io::{HttpClient as BaseHttpClient, HttpClientRequest, HttpEndpoint};
use nanofish::client::stream::{BodySink, StreamedResponse};
use nanofish::{error::Error, header::HttpHeader, method::HttpMethod, response::HttpResponse};

const NO_TLS: Error = Error::UnsupportedScheme("https (TLS support not enabled)");

impl<
    const TCP_RX: usize,
    const TCP_TX: usize,
    const TLS_READ: usize,
    const TLS_WRITE: usize,
    const RQ: usize,
> EmbassyHttpClient<'_, TCP_RX, TCP_TX, TLS_READ, TLS_WRITE, RQ>
{
    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
    #[expect(clippy::unused_async, reason = "same signature as the TLS variant")]
    pub(super) async fn make_https_request<'b>(
        &self,
        _method: HttpMethod,
        _endpoint: &HttpEndpoint<'_>,
        _headers: &[HttpHeader<'_>],
        _body: Option<&[u8]>,
        _response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error> {
        Err(NO_TLS)
    }

    #[expect(
        clippy::future_not_send,
        reason = "Embassy sockets are not `Send`; embedded executors run tasks on one thread"
    )]
    #[expect(clippy::unused_async, reason = "same signature as the TLS variant")]
    pub(super) async fn https_streaming<K: BodySink>(
        &self,
        _endpoint: &HttpEndpoint<'_>,
        _client: &BaseHttpClient<RQ>,
        _request: HttpClientRequest<'_>,
        _buffer: &mut [u8],
        _sink: &mut K,
    ) -> Result<StreamedResponse, Error> {
        Err(NO_TLS)
    }
}
