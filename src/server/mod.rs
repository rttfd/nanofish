//! The HTTP server: a handler answers requests, and a runner feeds it
//! connections.
//!
//! # Handlers and runners
//!
//! An [`HttpHandler`](nanofish::HttpHandler) returns a whole
//! [`ServerResponse`], which the server frames and sends. A
//! [`StreamingHandler`] writes its response through a [`ResponseWriter`]
//! instead, for bodies that are large or produced as they go.
//!
//! | Runner | Feature | Serves |
//! |---|---|---|
//! | [`HttpServer::handle_connection`] / [`handle_http_connection`] | none | One request on a stream you accepted |
//! | [`serve_http_connection_with_sizes`] | none | Up to `max_requests` on one stream (keep-alive) |
//! | [`handle_http_stream_connection`] | none | One request, with a [`StreamingHandler`] |
//! | `EmbassyHttpServer` | `embassy` | Accepts on a port, with timeouts, forever |
//!
//! `REQ_SIZE` is the connection buffer: the request head is read into it,
//! and the rest is the room the body is read through. A response from an
//! [`HttpHandler`](nanofish::HttpHandler) is serialized into a buffer of
//! `MAX_RESPONSE_SIZE`; a [`StreamingHandler`] only needs room for the head.
//!
//! Malformed or oversized requests are answered before the handler runs:
//! `400` for bad syntax or framing, `431` for headers that do not fit, `501`
//! for an unsupported transfer coding. A handler error is answered with
//! `500`. The server speaks plain HTTP: run it over a TLS stream you bring,
//! or terminate TLS in front of the device.
//!
//! # Requests and responses
//!
//! ```rust
//! use embedded_io_async::{Read, Write};
//! use nanofish::{
//!     Error, HttpHandler, HttpRequest, RequestBody, ServerResponse, ServerResponseBuilder,
//!     StatusCode,
//! };
//!
//! struct Api;
//!
//! impl HttpHandler for Api {
//!     async fn handle_request<S: Read + Write>(
//!         &mut self,
//!         request: &HttpRequest<'_>,
//!         _body: &mut RequestBody<'_, S>,
//!     ) -> Result<ServerResponse<'_>, Error> {
//!         if let Some(params) = request.match_path::<1>("/led/:id") {
//!             let _id = params.get("id");
//!             return ServerResponseBuilder::new().status(StatusCode::NoContent).build();
//!         }
//!         if request.if_none_match("\"v7\"") {
//!             return ServerResponseBuilder::new()
//!                 .status(StatusCode::NotModified)
//!                 .etag("\"v7\"")?
//!                 .build();
//!         }
//!         ServerResponseBuilder::new().etag("\"v7\"")?.text("hello").build()
//!     }
//! }
//! ```
//!
//! [`ServerResponseBuilder`] sets the status, headers and body (`text`,
//! `json`, `problem_json`, `binary`); `build` drops repeated headers, keeping
//! the last, except `Set-Cookie`. A `HEAD` request gets the headers without
//! the body.
//!
//! # Request bodies
//!
//! The server reads only the request head before it calls the handler, so
//! `request.body` is empty. The body is read through the [`RequestBody`]
//! the handler receives, `Content-Length` or chunked alike, without framing:
//!
//! - [`RequestBody::all`] returns the whole body as one slice, decoded in
//!   place. A body that does not fit is answered with `413`.
//! - [`RequestBody::chunk`] returns the body piece by piece, borrowed from the
//!   connection buffer, so an upload of any size passes through a small
//!   buffer.
//!
//! ```rust
//! use embedded_io_async::{Read, Write};
//! use nanofish::{
//!     Error, HttpHandler, HttpRequest, RequestBody, ServerResponse, ServerResponseBuilder,
//!     StatusCode,
//! };
//!
//! struct Upload {
//!     received: usize,
//! }
//!
//! impl HttpHandler for Upload {
//!     async fn handle_request<S: Read + Write>(
//!         &mut self,
//!         _request: &HttpRequest<'_>,
//!         body: &mut RequestBody<'_, S>,
//!     ) -> Result<ServerResponse<'_>, Error> {
//!         while let Some(piece) = body.chunk().await? {
//!             self.received += piece.len(); // or write it to flash
//!         }
//!         ServerResponseBuilder::new().status(StatusCode::NoContent).build()
//!     }
//! }
//! ```
//!
//! What the handler leaves unread is discarded after the response (up to
//! 16 KiB) and the connection closes. `Expect: 100-continue` is answered
//! when the handler first reads the body, so a handler that rejects the
//! request never invites it. A large head leaves less room for
//! [`RequestBody::all`].
//!
//! # Streaming responses
//!
//! A [`StreamingHandler`] sends the head with [`ResponseWriter::send_head`]
//! and then writes the body: with [`BodyLength::Known`] as
//! `Content-Length`, with [`BodyLength::Unknown`] as chunks. Its request body
//! is a [`BodyReader`], read by passing the writer, which owns the connection.
//! This handler echoes an upload back as it arrives:
//!
//! ```rust
//! use embedded_io_async::{Read, Write};
//! use nanofish::{
//!     BodyLength, BodyReader, Error, HttpRequest, ResponseWriter, StatusCode, StreamingHandler,
//! };
//!
//! struct Echo;
//!
//! impl StreamingHandler for Echo {
//!     async fn handle<S: Read + Write, const HEAD_SIZE: usize>(
//!         &mut self,
//!         _request: &HttpRequest<'_>,
//!         body: &mut BodyReader<'_>,
//!         response: &mut ResponseWriter<'_, S, HEAD_SIZE>,
//!     ) -> Result<(), Error> {
//!         response.send_head(StatusCode::Ok, &[], BodyLength::Unknown).await?;
//!         while let Some(piece) = body.chunk(response).await? {
//!             response.write(piece).await?;
//!         }
//!         Ok(()) // `finish` is called for you
//!     }
//! }
//! ```
//!
//! # Static files
//!
//! [`serve`] answers a request with a file from a [`FileStore`], reading it
//! through a small buffer, so a file of any size goes out without a heap. It
//! returns `false` for a path the store does not have, so the handler can go
//! on to its API routes. It handles `GET`/`HEAD`, `ETag` revalidation, gzip
//! variants and content types; see its documentation for the details.
//! [`StaticSite`] keeps the files in the image; a littlefs or FAT adapter
//! implements [`FileStore`] in a few lines, and its files can change without
//! reflashing.
//!
//! ```rust
//! use embedded_io_async::{Read, Write};
//! use nanofish::{
//!     BodyReader, Error, HttpRequest, ResponseWriter, StaticFile, StaticSite, StreamingHandler,
//!     serve,
//! };
//!
//! static SITE: StaticSite<1> = StaticSite::new([StaticFile {
//!     path: "/",
//!     content_type: Some("text/html"),
//!     body: b"<h1>hello</h1>",
//!     etag: "\"i1\"",
//!     gzip: None,
//! }]);
//!
//! struct Site;
//!
//! impl StreamingHandler for Site {
//!     async fn handle<S: Read + Write, const HEAD_SIZE: usize>(
//!         &mut self,
//!         request: &HttpRequest<'_>,
//!         _body: &mut BodyReader<'_>,
//!         response: &mut ResponseWriter<'_, S, HEAD_SIZE>,
//!     ) -> Result<(), Error> {
//!         let mut buf = [0u8; 512];
//!         if serve(&SITE, request, response, &mut buf).await? {
//!             return Ok(());
//!         }
//!         // Not a static file: API routes follow.
//!         Ok(())
//!     }
//! }
//! ```

pub(crate) mod body;
#[cfg(feature = "embassy")]
mod embassy;
mod io;
pub(crate) mod response;
mod static_site;
mod stream;

pub use body::{BodyReader, RequestBody};
#[cfg(feature = "embassy")]
pub use embassy::{
    DefaultEmbassyHttpServer, EmbassyHttpServer, ServerTimeouts, SmallEmbassyHttpServer,
};
pub use io::{
    DefaultHttpServer, HttpServer, SmallHttpServer, handle_http_connection,
    handle_http_connection_with_sizes, serve_http_connection_with_sizes,
};
pub use response::{ServerResponse, ServerResponseBuilder};
pub use static_site::{
    FileStat, FileStore, StaticFile, StaticSite, mime_for_extension, serve, serve_with,
};
pub use stream::{
    BodyLength, ResponseWriter, StreamingHandler, handle_http_stream_connection,
    handle_http_stream_connection_with_sizes,
};
