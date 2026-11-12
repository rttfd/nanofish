//! The HTTP client: pick a client for your transport, then send requests
//! through a buffer you own.
//!
//! # Choosing a client
//!
//! | Client | Feature | You provide |
//! |---|---|---|
//! | [`HttpClient`] | none | A connected `embedded-io-async` stream |
//! | `HttpTlsClient` | `tls` | A connected stream, an RNG and a root CA |
//! | `EmbassyHttpClient` | `embassy` | An Embassy `Stack`; DNS, TCP and timeouts are handled |
//! | `EmbassyHttpSession` | `embassy` | The same, for one server over a kept-alive connection |
//!
//! With `smoltcp`, wrap a connected socket in
//! `SmolTcpStream` and use [`HttpClient`]. The
//! Embassy types document their own examples.
//!
//! # Buffers
//!
//! A response is read into the buffer you pass and parsed in place:
//! [`HttpResponse`](nanofish::HttpResponse) borrows its headers and body from
//! that buffer, and the call also returns how many bytes it used. The
//! buffer must hold the whole response, head and body. For a body that does
//! not fit, stream it to a [`BodySink`] instead. The request is built in a
//! separate buffer whose size is a const generic (`RQ`, 1 KiB in the default
//! clients); the `Small*` aliases halve it.
//!
//! # Building requests
//!
//! [`RequestBuilder`] collects the method, headers and body, and
//! [`HttpClient::send`] adds the destination. The
//! [`encode`](nanofish::encode) module builds query strings, form bodies and
//! `Authorization` values without allocating.
//!
//! ```rust
//! use embedded_io_async::{Read, Write};
//! use nanofish::encode::{QueryBuilder, bearer_auth};
//! use nanofish::{Error, HttpClient, RequestBuilder};
//!
//! async fn switch_on<S: Read + Write>(stream: &mut S) -> Result<(), Error> {
//!     let token = bearer_auth::<64>("s3cret")?;
//!     let mut query = QueryBuilder::<64>::new();
//!     query.pair("fade", "1")?.pair("name", "front door")?;
//!     let path = heapless::format!(128; "/led/3?{}", query.as_str())
//!         .map_err(|_| Error::BufferOverflow)?;
//!
//!     let request = RequestBuilder::put()
//!         .authorization(&token)
//!         .json(r#"{"on":true}"#);
//!     let mut buffer = [0u8; 1024];
//!     let (response, _) = HttpClient::new()
//!         .send(stream, "example.com", &path, &request, &mut buffer)
//!         .await?;
//!     assert!(response.is_success());
//!     Ok(())
//! }
//! ```
//!
//! # Streaming large responses
//!
//! [`HttpClient::request_streaming`] hands the body to a [`BodySink`] as it
//! arrives, so the buffer only has to hold the head and one piece of the
//! body. A chunked body is decoded on the fly, and a response that is cut
//! short is an error, never a silent partial download.
//!
//! ```rust
//! use embedded_io_async::{Read, Write};
//! use nanofish::client::BodySink;
//! use nanofish::{Error, HttpClient, HttpClientRequest, HttpMethod, HttpResponse, StatusCode};
//!
//! struct Flash {
//!     written: usize,
//! }
//!
//! impl BodySink for Flash {
//!     async fn head(&mut self, response: &HttpResponse<'_>) -> Result<(), Error> {
//!         // Refuse anything but a complete image before touching flash.
//!         if response.status_code != StatusCode::Ok {
//!             return Err(Error::InvalidResponse("unexpected status"));
//!         }
//!         Ok(())
//!     }
//!
//!     async fn chunk(&mut self, data: &[u8]) -> Result<(), Error> {
//!         // Write `data` at `self.written`, then:
//!         self.written += data.len();
//!         Ok(())
//!     }
//! }
//!
//! async fn download<S: Read + Write>(stream: &mut S) -> Result<usize, Error> {
//!     let request = HttpClientRequest {
//!         method: HttpMethod::GET,
//!         host: "example.com",
//!         path: "/firmware.bin",
//!         query: None,
//!         headers: &[],
//!         body: None,
//!     };
//!     let mut buffer = [0u8; 1024];
//!     let mut flash = Flash { written: 0 };
//!     let streamed = HttpClient::new()
//!         .request_streaming(stream, request, &mut buffer, &mut flash)
//!         .await?;
//!     Ok(streamed.body_len)
//! }
//! ```
//!
//! # Uploading without buffering
//!
//! [`HttpClient::request_upload`] sends a body whose length is not known in
//! advance, with chunked transfer coding: each piece a [`BodySource`] yields
//! goes out as one chunk. [`SliceSource`] sends a slice in bounded pieces;
//! your own source can yield one flash page or SPI transaction at a time.
//!
//! ```rust
//! use embedded_io_async::{Read, Write};
//! use nanofish::client::SliceSource;
//! use nanofish::{Error, HttpClient, HttpClientRequest, HttpMethod};
//!
//! async fn upload<S: Read + Write>(stream: &mut S, image: &[u8]) -> Result<(), Error> {
//!     let request = HttpClientRequest {
//!         method: HttpMethod::PUT,
//!         host: "example.com",
//!         path: "/fw/7",
//!         query: None,
//!         headers: &[],
//!         body: None, // the body comes from the source
//!     };
//!     let mut source = SliceSource::new(image, 4096);
//!     let mut buffer = [0u8; 1024];
//!     HttpClient::new()
//!         .request_upload(stream, request, &mut source, &mut buffer)
//!         .await?;
//!     Ok(())
//! }
//! ```
//!
//! Once the head is written the source is consumed, so a failure mid-upload
//! is final: retries only repeat reads, and redirects are not followed. No
//! `Expect: 100-continue` is sent. A server that answers
//! `411 Length Required` does not accept chunked uploads at all.
//!
//! # Reusing connections
//!
//! [`HttpClient::exchange`] returns, with the response, whether the stream
//! can carry another request; `EmbassyHttpSession`
//! does this for you and reconnects when the server closes the connection.
//!
//! # Redirects
//!
//! Redirects are returned to the caller unless
//! [`HttpClientOptions::max_redirects`](nanofish::HttpClientOptions::max_redirects)
//! is set; the Embassy client then follows `301`, `302`, `303`, `307` and
//! `308`, reusing the response buffer. A `POST` answered by `301`, `302` or
//! `303` is repeated as a `GET` without its body; `Authorization` and
//! `Cookie` are dropped when a redirect leaves the origin; a redirect from
//! `https` to `http` is refused.

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
mod upload;

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
pub use tls::{
    DefaultHttpTlsClient, HttpTlsClient, SmallHttpTlsClient, TlsVerification, TlsVerifier,
};
pub use upload::{BodySource, SliceSource};
