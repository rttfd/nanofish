//! Static file serving: HTTP semantics in [`serve`], file bytes from any
//! [`FileStore`].
//!
//! The store decides where the bytes live. [`StaticSite`] embeds them in the
//! image (a const table of `&'static` slices); a filesystem adapter (littlefs,
//! SD/FatFs) implements the same trait, and its files update at runtime,
//! without recompiling or reflashing. The serving logic — method check, index
//! mapping, `ETag` revalidation, gzip negotiation, content types — is shared
//! by every store and tested here against an in-memory table.

use core::fmt::Write as _;
use embedded_io_async::Write;
use heapless::{String, Vec};
use nanofish::{
    error::Error,
    header::{HttpHeader, headers, mime_types},
    method::HttpMethod,
    protocol::MAX_HEADERS,
    request::{HttpRequest, etag_matches},
    server::stream::{BodyLength, ResponseWriter},
    status_code::StatusCode,
};

/// Paths longer than this fall through as unknown; they cannot map to an
/// `index.html` anyway.
const MAX_INDEX_PATH: usize = 96;

/// Length and metadata of one file, as a [`FileStore`] reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStat<'a> {
    /// The exact body length, announced as `Content-Length`.
    pub len: usize,
    /// A strong entity tag, quotes included (e.g. `"v1"`); `None` turns
    /// revalidation off for this file.
    pub etag: Option<&'a str>,
    /// The content type; `None` derives it from the path's extension.
    pub content_type: Option<&'a str>,
}

/// Where the bytes of a static site live.
///
/// [`StaticSite`] implements it for a table of files flashed into the image.
/// A filesystem (littlefs, SD/FatFs) implements it in user code — `stat`
/// describes a file, `read` fills `buf` starting at `offset` and returns how
/// many bytes it filled — and its files update at runtime, without a rebuild.
///
/// Paths are URL paths starting with `/` and without `..` segments
/// ([`serve`] rejects anything else before the store sees it); implementors
/// must refuse them too. `stat_gzip` describes the pre-compressed variant of
/// the same logical file (`foo.js.gz`); `None` when the store keeps none.
#[expect(
    async_fn_in_trait,
    reason = "no_std embedded executors are single-threaded, so no `Send` bound is wanted"
)]
pub trait FileStore {
    /// Describe the file at `path`, or `None` when the store has no such file.
    #[must_use]
    fn stat(&self, path: &str) -> Option<FileStat<'_>>;

    /// Describe the pre-compressed variant at `path`, or `None`.
    #[must_use]
    fn stat_gzip(&self, path: &str) -> Option<FileStat<'_>>;

    /// Fill `buf` with the file's bytes starting at `offset`, returning how
    /// many bytes were filled.
    ///
    /// Returning `0` before `len` bytes were served breaks the contract and
    /// fails the response.
    ///
    /// # Errors
    ///
    /// Returns the store's error; the response is aborted.
    async fn read(
        &self,
        path: &str,
        gzip: bool,
        offset: usize,
        buf: &mut [u8],
    ) -> Result<usize, Error>;
}

/// One file of a [`StaticSite`], embedded in the image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticFile {
    /// The request path it answers, e.g. `/app.js`.
    pub path: &'static str,
    /// The content type; `None` derives it from the path's extension.
    pub content_type: Option<&'static str>,
    /// The file's bytes.
    pub body: &'static [u8],
    /// A strong entity tag, quotes included (e.g. `"j1"`).
    pub etag: &'static str,
    /// The pre-compressed variant, served when the client accepts gzip.
    pub gzip: Option<&'static [u8]>,
}

/// A static site embedded in the image: a const table of [`StaticFile`]s.
///
/// Used through the [`FileStore`] implementation, inside a
/// [`StreamingHandler`](nanofish::StreamingHandler):
///
/// ```rust,ignore
/// static SITE: StaticSite<2> = StaticSite::new([
///     StaticFile { path: "/", content_type: Some("text/html"), body: INDEX,
///                 etag: "\"i1\"", gzip: None },
///     StaticFile { path: "/app.js", content_type: None, body: APP,
///                 etag: "\"a1\"", gzip: Some(APP_GZ) },
/// ]);
/// ```
///
/// Changing any of it means rebuilding and reflashing: the bytes are the
/// image. A [`FileStore`] over a filesystem updates without either.
pub struct StaticSite<const N: usize> {
    /// The files, in any order.
    pub files: [StaticFile; N],
}

impl<const N: usize> StaticSite<N> {
    /// A site holding `files`.
    #[must_use]
    pub const fn new(files: [StaticFile; N]) -> Self {
        Self { files }
    }

    fn find(&self, path: &str) -> Option<&StaticFile> {
        self.files.iter().find(|file| file.path == path)
    }
}

impl<const N: usize> FileStore for StaticSite<N> {
    fn stat(&self, path: &str) -> Option<FileStat<'_>> {
        self.find(path).map(|file| FileStat {
            len: file.body.len(),
            etag: Some(file.etag),
            content_type: file.content_type,
        })
    }

    fn stat_gzip(&self, path: &str) -> Option<FileStat<'_>> {
        self.find(path).and_then(|file| {
            file.gzip.map(|gzip| FileStat {
                len: gzip.len(),
                etag: Some(file.etag),
                content_type: None,
            })
        })
    }

    async fn read(
        &self,
        path: &str,
        gzip: bool,
        offset: usize,
        buf: &mut [u8],
    ) -> Result<usize, Error> {
        let file = self
            .find(path)
            .ok_or(Error::InvalidResponse("No such file"))?;
        let body = if gzip {
            file.gzip.ok_or(Error::InvalidResponse("No gzip variant"))?
        } else {
            file.body
        };
        let end = (offset + buf.len()).min(body.len());
        let slice = body
            .get(offset..end)
            .ok_or(Error::InvalidResponse("Read past the end of the file"))?;
        buf[..slice.len()].copy_from_slice(slice);
        Ok(slice.len())
    }
}

/// The `Content-Type` for a file extension, `application/octet-stream` when
/// unknown.
///
/// Covers a browser UI's needs: html, css, js, mjs, json, svg, png, jpg, ico,
/// woff2, wasm, txt, map.
#[must_use]
pub fn mime_for_extension(extension: &str) -> &'static str {
    match extension {
        "html" => mime_types::HTML,
        "css" => "text/css",
        "js" | "mjs" => "text/javascript",
        "json" | "map" => mime_types::JSON,
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" => "image/jpeg",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "wasm" => "application/wasm",
        "txt" => mime_types::TEXT,
        _ => mime_types::BINARY,
    }
}

/// The content type for a path, from its extension.
fn mime_for_path(path: &str) -> &'static str {
    path.rsplit('/')
        .next()
        .and_then(|segment| segment.rsplit_once('.'))
        .map_or(mime_types::BINARY, |(_, extension)| {
            mime_for_extension(extension)
        })
}

/// Whether the request accepts a gzip-coded body.
fn accepts_gzip(request: &HttpRequest<'_>) -> bool {
    request
        .header(headers::ACCEPT_ENCODING)
        .is_some_and(|value| {
            value.split(',').any(|coding| {
                coding
                    .split(';')
                    .next()
                    .is_some_and(|token| token.trim().eq_ignore_ascii_case("gzip"))
            })
        })
}

/// Resolve the request path to a path the store has: the path itself, or its
/// directory index (`/` and `/x` map to `index.html`), with the file it
/// describes. Unsafe paths and overlong paths resolve to nothing.
fn resolve_path<'a, S: FileStore>(
    store: &'a S,
    path: &str,
) -> Option<(String<MAX_INDEX_PATH>, FileStat<'a>)> {
    if !path.starts_with('/') || path.split('/').any(|segment| segment == "..") {
        return None;
    }
    let mut resolved = String::<MAX_INDEX_PATH>::new();
    write!(resolved, "{path}").ok()?;
    if let Some(file) = store.stat(&resolved) {
        return Some((resolved, file));
    }
    if !path.ends_with('/') {
        resolved.push('/').ok()?;
    }
    resolved.push_str("index.html").ok()?;
    store.stat(&resolved).map(|file| (resolved, file))
}

/// Serve `request` from `store` through `response`, reading file bytes
/// through `buf`.
///
/// The buffer bounds one read from the store; files larger than it stream in
/// pieces. [`serve_with`] adds headers such as `Cache-Control`.
///
/// Returns `Ok(true)` when a response was sent. Returns `Ok(false)` when the
/// store has no such path — or the path is unsafe — so the caller can fall
/// through to its API routes:
///
/// ```rust,ignore
/// if serve(&SITE, request, response, &mut buf).await? {
///     return Ok(());
/// }
/// // API routes follow
/// ```
///
/// Semantics: `GET` and `HEAD` only (anything else gets `405`); `/` and `/x`
/// map to `index.html`; a matching `If-None-Match` gets `304`; the gzip
/// variant is served with `Content-Encoding: gzip` when the client accepts
/// gzip and the store has one. The file's plain `ETag` identifies both
/// variants, as proxies do. A `HEAD` is answered with the head only, but the
/// bytes are still read from the store.
///
/// # Errors
///
/// - The errors of [`ResponseWriter::send_head`], [`ResponseWriter::write`]
///   and [`ResponseWriter::finish`], plus the store's read errors.
/// - [`Error::InvalidResponse`] when a store ends a file before its `len`.
/// - [`Error::BufferOverflow`] when the file is non-empty but `buf` is empty.
#[expect(
    clippy::future_not_send,
    reason = "no_std embedded executors are single-threaded, so no `Send` bound is wanted"
)]
pub async fn serve<S, W, const HEAD_SIZE: usize>(
    store: &S,
    request: &HttpRequest<'_>,
    response: &mut ResponseWriter<'_, W, HEAD_SIZE>,
    buf: &mut [u8],
) -> Result<bool, Error>
where
    S: FileStore,
    W: Write,
{
    serve_with(store, request, response, buf, &[]).await
}

/// [`serve`] with extra response headers, such as `Cache-Control`.
///
/// The headers go on every response the site sends, `405` and `304`
/// included. Framing headers are refused by the writer.
///
/// # Errors
///
/// Returns the same errors as [`serve`].
#[expect(
    clippy::future_not_send,
    reason = "no_std embedded executors are single-threaded, so no `Send` bound is wanted"
)]
pub async fn serve_with<S, W, const HEAD_SIZE: usize>(
    store: &S,
    request: &HttpRequest<'_>,
    response: &mut ResponseWriter<'_, W, HEAD_SIZE>,
    buf: &mut [u8],
    headers: &[HttpHeader<'_>],
) -> Result<bool, Error>
where
    S: FileStore,
    W: Write,
{
    if request.method != HttpMethod::GET && request.method != HttpMethod::HEAD {
        let mut all = Vec::<HttpHeader<'_>, MAX_HEADERS>::new();
        for header in headers {
            all.push(header.clone())
                .map_err(|_| Error::HeaderError("Too many headers"))?;
        }
        all.push(HttpHeader::new("Allow", "GET, HEAD"))
            .map_err(|_| Error::HeaderError("Too many headers"))?;
        response
            .send_head(StatusCode::MethodNotAllowed, &all, BodyLength::Known(0))
            .await?;
        return Ok(true);
    }

    let Some((path, file)) = resolve_path(store, request.route_path()) else {
        return Ok(false);
    };

    if let (Some(if_none_match), Some(etag)) = (request.header(headers::IF_NONE_MATCH), file.etag)
        && etag_matches(if_none_match, etag)
    {
        let mut all = Vec::<HttpHeader<'_>, MAX_HEADERS>::new();
        for header in headers {
            all.push(header.clone())
                .map_err(|_| Error::HeaderError("Too many headers"))?;
        }
        all.push(HttpHeader::etag(etag))
            .map_err(|_| Error::HeaderError("Too many headers"))?;
        response
            .send_head(StatusCode::NotModified, &all, BodyLength::Known(0))
            .await?;
        return Ok(true);
    }

    let gzip = if accepts_gzip(request) {
        store.stat_gzip(&path)
    } else {
        None
    };
    let len = gzip.map_or(file.len, |gzip| gzip.len);
    if len > 0 && buf.is_empty() {
        return Err(Error::BufferOverflow);
    }

    let mut all = Vec::<HttpHeader<'_>, MAX_HEADERS>::new();
    for header in headers {
        all.push(header.clone())
            .map_err(|_| Error::HeaderError("Too many headers"))?;
    }
    let content_type = file.content_type.unwrap_or_else(|| mime_for_path(&path));
    all.push(HttpHeader::content_type(content_type))
        .map_err(|_| Error::HeaderError("Too many headers"))?;
    if let Some(etag) = file.etag {
        all.push(HttpHeader::etag(etag))
            .map_err(|_| Error::HeaderError("Too many headers"))?;
    }
    if gzip.is_some() {
        all.push(HttpHeader::new("Content-Encoding", "gzip"))
            .map_err(|_| Error::HeaderError("Too many headers"))?;
    }

    response
        .send_head(StatusCode::Ok, &all, BodyLength::Known(len))
        .await?;

    let serving_gzip = gzip.is_some();
    let mut offset = 0;
    while offset < len {
        let n = store.read(&path, serving_gzip, offset, buf).await?;
        if n == 0 {
            return Err(Error::InvalidResponse(
                "Static file ended before its length",
            ));
        }
        response.write(&buf[..n]).await?;
        offset += n;
    }
    response.finish().await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use core::convert::Infallible;
    use embedded_io_async::ErrorType;
    use heapless::Vec;
    use nanofish::error::Error;
    use nanofish::header::HttpHeader;
    use nanofish::method::HttpMethod;
    use nanofish::request::HttpRequest;
    use nanofish::server::static_site::*;
    use nanofish::server::stream::ResponseWriter;

    struct MockStream<const OUT: usize> {
        output: Vec<u8, OUT>,
    }

    impl<const OUT: usize> MockStream<OUT> {
        fn new() -> Self {
            Self { output: Vec::new() }
        }

        fn output(&self) -> &str {
            core::str::from_utf8(&self.output).unwrap()
        }
    }

    impl<const OUT: usize> ErrorType for MockStream<OUT> {
        type Error = Infallible;
    }

    impl<const OUT: usize> embedded_io_async::Write for MockStream<OUT> {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            self.output.extend_from_slice(buf).unwrap();
            Ok(buf.len())
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    const BIG: &[u8] = &[b'x'; 300];

    static SITE: StaticSite<5> = StaticSite::new([
        StaticFile {
            path: "/index.html",
            content_type: Some("text/html"),
            body: b"<h1>hi</h1>",
            etag: "\"i1\"",
            gzip: None,
        },
        StaticFile {
            path: "/app.js",
            content_type: None,
            body: b"console.log(1)",
            etag: "\"j1\"",
            gzip: Some(b"<gz:app.js>"),
        },
        StaticFile {
            path: "/sub/index.html",
            content_type: Some("text/html"),
            body: b"<h1>sub</h1>",
            etag: "\"s1\"",
            gzip: None,
        },
        StaticFile {
            path: "/logo.svg",
            content_type: None,
            body: b"<svg/>",
            etag: "\"l1\"",
            gzip: None,
        },
        StaticFile {
            path: "/empty.txt",
            content_type: None,
            body: b"",
            etag: "\"e1\"",
            gzip: None,
        },
    ]);

    fn serve_path(head: &str) -> (Result<bool, Error>, std::string::String) {
        serve_path_with(head, &[])
    }

    fn serve_path_with(
        head: &str,
        headers: &[HttpHeader<'_>],
    ) -> (Result<bool, Error>, std::string::String) {
        let request = HttpRequest::parse_from(head, b"").unwrap();
        let mut stream = MockStream::<1024>::new();
        let mut writer = ResponseWriter::<_, 256>::new(&mut stream, &request);
        let mut buf = [0u8; 64];
        let result = if headers.is_empty() {
            futures_lite::future::block_on(serve(&SITE, &request, &mut writer, &mut buf))
        } else {
            futures_lite::future::block_on(serve_with(
                &SITE,
                &request,
                &mut writer,
                &mut buf,
                headers,
            ))
        };
        (result, stream.output().to_string())
    }

    #[test]
    fn test_get_serves_file_with_type_etag_and_length() {
        let (result, output) = serve_path("GET /app.js HTTP/1.1\r\nHost: h\r\n\r\n");
        assert!(result.unwrap());
        assert_eq!(
            output,
            "HTTP/1.1 200 OK\r\n\
             Content-Type: text/javascript\r\n\
             ETag: \"j1\"\r\n\
             Content-Length: 14\r\n\
             Connection: close\r\n\
             \r\n\
             console.log(1)"
        );
    }

    #[test]
    fn test_content_type_derives_from_the_extension() {
        let (result, output) = serve_path("GET /logo.svg HTTP/1.1\r\nHost: h\r\n\r\n");
        result.unwrap();
        assert!(
            output.contains("Content-Type: image/svg+xml\r\n"),
            "{output}"
        );
    }

    #[test]
    fn test_root_and_directory_map_to_index_html() {
        let (result, output) = serve_path("GET / HTTP/1.1\r\nHost: h\r\n\r\n");
        assert!(result.unwrap());
        assert!(output.contains("Content-Length: 11\r\n"), "{output}");
        assert!(output.ends_with("<h1>hi</h1>"), "{output}");

        let (result, output) = serve_path("GET /sub HTTP/1.1\r\nHost: h\r\n\r\n");
        assert!(result.unwrap());
        assert!(output.ends_with("<h1>sub</h1>"), "{output}");
    }

    #[test]
    fn test_unknown_path_falls_through_without_a_response() {
        let (result, output) = serve_path("GET /missing.js HTTP/1.1\r\nHost: h\r\n\r\n");
        assert!(!result.unwrap());
        assert!(output.is_empty());
    }

    #[test]
    fn test_conditional_get_answers_304() {
        let (result, output) =
            serve_path("GET /app.js HTTP/1.1\r\nHost: h\r\nIf-None-Match: \"j1\"\r\n\r\n");
        assert!(result.unwrap());
        assert_eq!(
            output,
            "HTTP/1.1 304 Not Modified\r\n\
             ETag: \"j1\"\r\n\
             Content-Length: 0\r\n\
             Connection: close\r\n\
             \r\n"
        );
    }

    #[test]
    fn test_stale_etag_still_serves_the_file() {
        let (result, output) =
            serve_path("GET /app.js HTTP/1.1\r\nHost: h\r\nIf-None-Match: \"old\"\r\n\r\n");
        result.unwrap();
        assert!(output.starts_with("HTTP/1.1 200 OK\r\n"), "{output}");
        assert!(output.ends_with("console.log(1)"), "{output}");
    }

    #[test]
    fn test_head_sends_the_head_only() {
        let (result, output) = serve_path("HEAD /app.js HTTP/1.1\r\nHost: h\r\n\r\n");
        result.unwrap();
        assert!(
            output.contains("Content-Type: text/javascript\r\n"),
            "{output}"
        );
        assert!(output.contains("Content-Length: 14\r\n"), "{output}");
        assert!(output.ends_with("\r\n\r\n"), "{output}");
    }

    #[test]
    fn test_gzip_variant_when_accepted() {
        let (result, output) =
            serve_path("GET /app.js HTTP/1.1\r\nHost: h\r\nAccept-Encoding: gzip\r\n\r\n");
        result.unwrap();
        assert!(
            output.contains("Content-Type: text/javascript\r\n"),
            "{output}"
        );
        assert!(output.contains("Content-Encoding: gzip\r\n"), "{output}");
        // The gzip variant's length, not the plain file's.
        assert!(output.contains("Content-Length: 11\r\n"), "{output}");
        assert!(output.ends_with("<gz:app.js>"), "{output}");
    }

    #[test]
    fn test_gzip_q_values_do_not_hide_the_variant() {
        let (result, output) = serve_path(
            "GET /app.js HTTP/1.1\r\nHost: h\r\nAccept-Encoding: br, gzip;q=0.7\r\n\r\n",
        );
        result.unwrap();
        assert!(output.contains("Content-Encoding: gzip\r\n"), "{output}");
    }

    #[test]
    fn test_gzip_not_served_without_acceptance() {
        let (result, output) =
            serve_path("GET /app.js HTTP/1.1\r\nHost: h\r\nAccept-Encoding: br\r\n\r\n");
        result.unwrap();
        assert!(!output.contains("Content-Encoding"), "{output}");
        assert!(output.ends_with("console.log(1)"), "{output}");
    }

    #[test]
    fn test_gzip_lacking_store_serves_plain() {
        let (result, output) =
            serve_path("GET /index.html HTTP/1.1\r\nHost: h\r\nAccept-Encoding: gzip\r\n\r\n");
        result.unwrap();
        assert!(!output.contains("Content-Encoding"), "{output}");
        assert!(output.ends_with("<h1>hi</h1>"), "{output}");
    }

    #[test]
    fn test_other_methods_get_405() {
        for method in [HttpMethod::POST, HttpMethod::PUT, HttpMethod::DELETE] {
            let head =
                format!("{method:?} /app.js HTTP/1.1\r\nHost: h\r\nContent-Length: 0\r\n\r\n");
            let (result, output) = serve_path(&head);
            assert!(result.unwrap(), "{method:?}");
            assert!(
                output.starts_with("HTTP/1.1 405 Method Not Allowed\r\n"),
                "{method:?}"
            );
            assert!(output.contains("Allow: GET, HEAD\r\n"), "{method:?}");
            assert!(output.ends_with("\r\n\r\n"), "{method:?}");
        }
    }

    #[test]
    fn test_traversal_and_unsafe_paths_fall_through() {
        for path in ["/../secret", "/a/../secret", "app.js"] {
            let (result, output) = serve_path(&format!("GET {path} HTTP/1.1\r\nHost: h\r\n\r\n"));
            assert!(!result.unwrap(), "{path}");
            assert!(output.is_empty(), "{path}");
        }
    }

    #[test]
    fn test_body_larger_than_the_read_buffer_streams() {
        static BIG_SITE: StaticSite<1> = StaticSite::new([StaticFile {
            path: "/big.bin",
            content_type: None,
            body: BIG,
            etag: "\"b1\"",
            gzip: None,
        }]);
        let request =
            HttpRequest::parse_from("GET /big.bin HTTP/1.1\r\nHost: h\r\n\r\n", b"").unwrap();
        let mut stream = MockStream::<1024>::new();
        let mut writer = ResponseWriter::<_, 256>::new(&mut stream, &request);
        let mut buf = [0u8; 64];
        let result =
            futures_lite::future::block_on(serve(&BIG_SITE, &request, &mut writer, &mut buf));
        assert!(result.unwrap());
        let output = stream.output();
        assert!(output.contains("Content-Length: 300\r\n"), "{output}");
        let body_start = output.find("\r\n\r\n").unwrap() + 4;
        assert_eq!(&output.as_bytes()[body_start..], BIG);
    }

    #[test]
    fn test_empty_file_has_no_body() {
        let (result, output) = serve_path("GET /empty.txt HTTP/1.1\r\nHost: h\r\n\r\n");
        result.unwrap();
        assert!(output.contains("Content-Length: 0\r\n"), "{output}");
        assert!(
            output.ends_with("Content-Length: 0\r\nConnection: close\r\n\r\n"),
            "{output}"
        );
    }

    #[test]
    fn test_extra_headers_reach_every_response() {
        let cache = [HttpHeader::cache_control("max-age=3600")];
        let (result, output) = serve_path_with("GET /app.js HTTP/1.1\r\nHost: h\r\n\r\n", &cache);
        result.unwrap();
        assert!(
            output.contains("Cache-Control: max-age=3600\r\n"),
            "{output}"
        );

        let (result, output) = serve_path_with(
            "GET /app.js HTTP/1.1\r\nHost: h\r\nIf-None-Match: \"j1\"\r\n\r\n",
            &cache,
        );
        result.unwrap();
        assert!(output.starts_with("HTTP/1.1 304"), "{output}");
        assert!(
            output.contains("Cache-Control: max-age=3600\r\n"),
            "{output}"
        );

        let (result, output) =
            serve_path_with("DELETE /app.js HTTP/1.1\r\nHost: h\r\n\r\n", &cache);
        result.unwrap();
        assert!(output.starts_with("HTTP/1.1 405"), "{output}");
        assert!(
            output.contains("Cache-Control: max-age=3600\r\n"),
            "{output}"
        );
    }

    #[test]
    fn test_empty_read_buffer_is_refused_for_a_nonempty_file() {
        let request =
            HttpRequest::parse_from("GET /app.js HTTP/1.1\r\nHost: h\r\n\r\n", b"").unwrap();
        let mut stream = MockStream::<1024>::new();
        let mut writer = ResponseWriter::<_, 256>::new(&mut stream, &request);
        let result = futures_lite::future::block_on(serve(&SITE, &request, &mut writer, &mut []));
        assert!(matches!(result, Err(Error::BufferOverflow)));
    }

    #[test]
    fn test_mime_for_extension() {
        assert_eq!(mime_for_extension("html"), "text/html");
        assert_eq!(mime_for_extension("js"), "text/javascript");
        assert_eq!(mime_for_extension("mjs"), "text/javascript");
        assert_eq!(mime_for_extension("json"), "application/json");
        assert_eq!(mime_for_extension("map"), "application/json");
        assert_eq!(mime_for_extension("svg"), "image/svg+xml");
        assert_eq!(mime_for_extension("woff2"), "font/woff2");
        assert_eq!(mime_for_extension("wasm"), "application/wasm");
        assert_eq!(mime_for_extension("bin"), "application/octet-stream");
        assert_eq!(mime_for_extension(""), "application/octet-stream");
    }
}
