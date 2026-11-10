# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.16.0] - 2026-11-10

### Breaking changes

- The response type is split by direction. `HttpResponse` (with its text/binary `ResponseBody`) is now only the response the client receives. A handler returns a `ServerResponse`, whose body is plain bytes (`&[u8]`; text is `"…".as_bytes()` or a `b"…"` literal), built with `ServerResponseBuilder` (formerly `HttpResponseBuilder`, same methods). `build_bytes`, `build_bytes_for` and `build_head_bytes` moved to `ServerResponse`. `HttpHandler::handle_request` returns `Result<ServerResponse<'_>, Error>`.
- `HttpResponse::get_header` is renamed `header`, matching `HttpRequest::header` and `ServerResponse::header`.
- `Error` is `#[non_exhaustive]` and says which side failed: `InvalidRequest` for a request the server could not parse or refuses (it was `InvalidResponse`), `InvalidState` for an API used out of order such as `ResponseWriter` writes before the head or past the declared length (was `InvalidResponse`), and `FileError` for a `FileStore` that cannot produce a file (was `InvalidResponse`). `InvalidResponse` now only means a bad response from a server. `HttpRequest::parse_from`, `body_str` and `percent_decode` return `InvalidRequest`.
- The server parses requests strictly and answers `400` to what it used to accept: bare LF line endings, whitespace before a header's colon, folded header lines, header lines without a colon, extra tokens on the request line, and versions other than `HTTP/1.0` and `HTTP/1.1`.
- Requests with their own `Host` header, a `Content-Length` that disagrees with the body, or (outside a chunked upload) a `Transfer-Encoding` header are refused with `HeaderError`; so are server responses whose `Content-Length` disagrees with the body or that set `Transfer-Encoding` on a buffered body.
- `handle_http_connection` and `serve_http_connection_with_sizes` return the handler's error after answering `500`, instead of `Ok(())`.
- A handler that exceeds `ServerTimeouts::handler_timeout` gets `503 Service Unavailable` instead of `408 Request Timeout`.
- Static file paths are percent-decoded before lookup: `/my%20file.txt` finds `/my file.txt`, and a store keyed by encoded paths no longer matches.
- `HttpClientOptions::max_retries = 0` means one read attempt instead of none.

### Fixed

- `serve`/`serve_with` answered `405` to every method but `GET` and `HEAD` before looking at the path, so `POST /api/...` behind a static site never reached the API routes. The path is now resolved first: a path the store lacks returns `Ok(false)` for any method, and only the site's own files get `405`.
- `EmbassyHttpSession::request_upload` repeated an upload on a fresh connection when the server had dropped the idle one, but the `BodySource` was already consumed, so the repeat sent an empty body that the server accepted. Uploads are no longer repeated; the error is returned.
- The server handed a handler bytes past the end of its request as body: with a pipelined request in the same read, `Content-Length: 5` gave `hello` plus the next request, and a request without a body got the next request as its body. The body now ends where the framing says.
- The client took an interim `1xx` response (`100 Continue`, `103 Early Hints`) as the final response and left the real one in the socket, where the next request on a reused connection read it. Interim responses are skipped, in buffered and streaming requests.
- Chunked framing was parsed three times, by three parsers that disagreed: two accepted a size of `+5` or ` 5`, skipped whatever two bytes followed chunk data instead of requiring CRLF, and accepted a missing last chunk. One strict decoder now serves the server, the buffered client and the streaming client.
- Only the first `Transfer-Encoding` header was read, so `chunked` in one header and `gzip` in another passed as chunked. All of them now form one list, and `chunked` counts only as the final coding.
- Request heads are parsed strictly: a bare LF (which split one header into two for the handler but not for the framing), whitespace before a header's colon, folded lines, header lines without a colon, extra tokens on the request line and versions other than `HTTP/1.0`/`HTTP/1.1` get `400`.
- IPv6 literal URLs (`http://[::1]/`) never connected: the bracketed host was sent to DNS. The Embassy clients now resolve the bare address.
- A redirect that turns a `POST` into a `GET` dropped the body but kept a caller's `Content-Length`, so the server waited for bytes that never came. `Content-*` headers now go with the body.
- Files with a gzip variant are served with `Vary: Accept-Encoding`, so a cache does not hand gzip to a client that cannot decode it.
- The client kept bytes after a `Content-Length` body (or after the head of a body-less response) in `response.body`; the response now ends where its framing says.
- `handle_http_connection` and `serve_http_connection_with_sizes` swallowed a handler's error after sending the `500`, although the docs said it was returned.
- A handler that exceeded `ServerTimeouts::handler_timeout` was answered `408 Request Timeout`, which blames the client; the `503` now also closes the connection.
- `HttpResponseBuilder::build` deduplicated `Set-Cookie`, keeping only the last cookie. Every `Set-Cookie` is now kept.
- A response or request whose `Content-Length` header disagrees with its buffered body, or that carries `Transfer-Encoding` with a buffered body, is refused with `HeaderError` instead of being sent with contradictory framing. A request with its own `Host` header is refused too (a second `Host` makes servers answer `400`), and a caller's `Connection` header is no longer joined by a second `Connection: close`.
- `1xx`, `204` and `304` responses never carry a body or framing headers: `build_bytes` drops a body given to them, and `ResponseWriter` no longer sends `Content-Length: 0` on a `304` or chunked coding on a `204`.
- Formatting a length of 10¹⁰ or more could panic (a 10-digit buffer); it now holds every `usize`.
- `HttpClientOptions::max_retries = 0` failed every request without reading; `0` now means one attempt.
- Static file paths are percent-decoded before lookup (`/my%20file.txt` finds `/my file.txt`), and checked after decoding: `%2e%2e`, backslashes, empty segments and control characters can no longer reach a `FileStore`.

### Performance

- The server searches for the end of the request head only in new bytes, so a client sending one byte per read costs linear rather than quadratic time.
- The server writes a buffered response body straight from the handler's slice instead of copying it into the response buffer, and error responses use a small buffer of their own instead of a second response-sized one in every connection future.
- A `HEAD` request for a static file no longer reads the file from the store.
- Faster CRLF searches over response and request heads; the client decodes chunked coding only when the response uses it.
- `EmbassyHttpSession` parses each response once instead of twice, and redirect hops are decided from the status line and `Location` alone, so the final response is the only one parsed in full.

### Added

- `ServerResponse`, `ServerResponseBuilder`, and `Error::InvalidRequest`, `Error::InvalidState` and `Error::FileError` (see Breaking changes).
- `percent_decode_path`, which percent-decodes a path segment without turning `+` into a space, as `percent_decode` does for query components.
- `headers::SET_COOKIE`.

## [0.15.0] - 2026-11-10

### Added

- Chunked request upload, the fourth and last chunked piece (issue #59): `HttpClient::request_upload` and `exchange_upload` (each with a `_with_retry_delay` variant), `EmbassyHttpClient::request_upload` and `EmbassyHttpSession::request_upload` send a request body from a `BodySource` piece by piece, with chunked transfer coding — no `Content-Length`, no buffering, any method. `SliceSource` sends a slice in bounded pieces. A chunked upload is fully delimited, so keep-alive connections stay reusable. With this, chunked responses in both directions (0.11.4+) and chunked requests in both directions all work.
- Static file serving over any store: `serve`/`serve_with` answer `GET`/`HEAD` from a `FileStore` — index mapping (`/` and `/x` to `index.html`), strong-`ETag` revalidation (`304`), gzip negotiation with pre-compressed variants, content types from the path's extension, `405` for other methods, `Cache-Control`-style extra headers — reading file bytes through a caller buffer, so any file size serves without buffering. `StaticSite` embeds the files in the image as a const table; a `FileStore` over littlefs or SD/FatFs (a trait, implemented in user code) serves files that update at runtime, without a rebuild or reflash.

## [0.14.2] - 2026-11-10

### Fixed

- `EmbassyHttpServer` now waits for its FIN to go out before reusing the socket. It used to call `TcpSocket::close` and drop the socket straight away, before the network task could send the FIN, so clients that read until the connection closes hung until they timed out, and the client's own FIN was answered with an RST. A peer that never acknowledges is aborted after 2 seconds.
- `EmbassyHttpServer::serve` and `handle_http_connection` closed the connection after every response without saying so. They now send `Connection: close`, so an HTTP/1.1 client such as a browser stops sending its next request into a connection that is already gone (a reset). The keep-alive and streaming paths already did this.

### Documentation

- The TLS note no longer says the server is HTTP-only. The Embassy server accepts plain TCP, and the server core runs over any `embedded-io-async` stream, so a TLS stream you bring works too.
- Explained that each server task serves one connection at a time and that parallel clients (a browser loading a page) need one task per connection, with an example.

## [0.14.1] - 2026-10-31

### Added

- Criterion benchmarks for response parsing, request serving and the encoders (`make bench`, `make bench-save`, `make bench-compare`), and a `profiling` build profile for `samply` and `cargo flamegraph`.
- `AGENTS.md` with the crate's code rules, and `make check` and `make lint` targets.

### Changed

- The code follows the rules in `AGENTS.md`: it refers to itself as `nanofish::` instead of `crate::` and `super::`, gates TLS and Embassy code by module (`client/embassy/tls.rs`, `error/embassy.rs`, `error/tls.rs`) instead of inside files, lints tests and benchmarks too, and gives every lint exception a reason. There are no public API changes.

### Fixed

- `make clippy-all` and `make test-all` ignored a failure in any feature combination except the last one, so a broken combination could pass CI. They now stop at the first failure.

## [0.14.0] - 2026-10-30

### Added

- `RequestBuilder`: a fluent builder for the method, headers and body of a request (`header`, `authorization`, `json`, `form`, `body`), sent with `HttpClient::send`, `HttpTlsClient::send` or `EmbassyHttpClient::send`. Adding more than 16 headers is reported as `BufferOverflow` before anything is sent. `post_json` is a shorthand on the generic and Embassy clients.
- The `encode` module: allocation-free `percent_encode`, `base64_encode`, `basic_auth` and `bearer_auth` (for `Authorization` values), and `QueryBuilder` for query strings and `application/x-www-form-urlencoded` bodies.
- Redirect following for the Embassy clients: set `HttpClientOptions::max_redirects` (default `0`, off). Follows `301`, `302`, `303`, `307` and `308` with relative and absolute `Location` values, turns a `POST` answered by `301`, `302` or `303` into a `GET`, drops `Authorization`, `Cookie` and `Proxy-Authorization` when leaving the original origin, and refuses `https` to `http` downgrades.
- Streaming response bodies: `HttpClient::request_streaming`, `HttpTlsClient::request_streaming` and `EmbassyHttpClient::request_streaming` pass the body to a `BodySink` piece by piece, so the response buffer only has to hold the headers and one piece of the body. Chunked bodies are decoded on the fly and a truncated response is an error.
- Streaming server responses: `StreamingHandler` and `ResponseWriter` (`send_head`, `write`, `finish`) send large or incremental responses without building them in memory, with `Content-Length` or chunked framing chosen from `BodyLength`. Run them with `handle_http_stream_connection` or `EmbassyHttpServer::serve_streaming`.
- Connection reuse: `HttpClient::exchange` reports whether a stream can carry another request, and `EmbassyHttpSession` keeps one connection open, reconnecting by itself and repeating safe requests once when the server dropped it. On the server, `serve_http_connection_with_sizes` and `EmbassyHttpServer::with_keep_alive` serve several requests per connection (HTTP/1.1 keep-alive, no pipelining).
- `EmbassyHttpClient::request_with_tls` makes HTTPS requests with a caller-supplied `HttpTlsClient`, so Embassy users can verify or pin server certificates (`TlsVerifier` is re-exported). `EmbassyHttpClient::request` still does not verify.
- `HttpClientOptions::total_timeout` bounds a whole request on the Embassy clients (`Error::Timeout`), and `ServerTimeouts::connection_timeout` bounds a whole connection on the Embassy server, so a peer that trickles bytes cannot hold a socket forever.
- Server helpers: `HttpRequest::match_path` / `match_path` for `/led/:id` and trailing `*` patterns, `HttpRequest::if_none_match` / `etag_matches` and `HttpResponseBuilder::etag` for `304 Not Modified`, and `HEAD` requests now get their headers (with the `Content-Length` a `GET` would have) without the body.
- `HttpHeader::etag`, `HttpHeader::if_none_match` and the `ETAG`, `IF_NONE_MATCH`, `LOCATION` and `TRANSFER_ENCODING` header constants; `Debug` for `HttpResponse`.
- `HttpEndpoint::host_header()`, which builds the `Host` header value for an endpoint: the host, plus `:port` unless it is the default for the scheme.

### Changed

- **BREAKING**: `Error::TcpError` now carries the `embedded_io_async::ErrorKind` reported by the transport (`TcpError(ErrorKind)`), so callers can tell a reset from a timeout. Added `Error::Timeout`.
- **BREAKING**: `HttpClientOptions` has two new fields, `total_timeout: Option<TimeoutDuration>` and `max_redirects: usize`. Use `..HttpClientOptions::default()` when constructing it.
- **BREAKING**: `ServerTimeouts` has a new `connection_timeout` field and `ServerTimeouts::new` takes it as a fourth argument (default 120 seconds).
- Response completion is tracked incrementally, so each read costs only the newly arrived bytes instead of rescanning the whole buffer (quadratic for many small reads).
- The server now answers requests it rejects instead of silently closing the connection: `400 Bad Request` for a malformed request line or headers, `413 Request Entity Too Large` for a body that does not fit the request buffer, `431 Request Header Fields Too Large` for headers that do not fit, and `500 Internal Server Error` when the handler's response cannot be serialized (an invalid header or a response larger than the response buffer). `handle_http_connection()` and `HttpServer::handle_connection()` still return the original error. Nothing is sent when the stream itself fails or the peer closes the connection mid-request. These responses carry `Connection: close`.
- The server rejects a request whose `Content-Length` cannot fit in the request buffer as soon as the headers arrive, instead of reading until the buffer fills. It now reports `BufferOverflow` where it used to report `InvalidResponse("Incomplete request")` when the peer closed first.
- Added `StatusCode::RequestHeaderFieldsTooLarge` (431). **BREAKING** for exhaustive matches on `StatusCode`, and `StatusCode::from(431)` now returns it instead of `Other(431)`.
- **BREAKING**: `HttpEndpoint::path` no longer includes the query string, and is always non-empty and starts with `/`. The query is in the new `HttpEndpoint::query: Option<&str>` field (without the leading `?`). `parse_endpoint("http://host/a?x=1")` now yields path `/a` and query `x=1`, and `http://host?x=1` yields path `/` and query `x=1`. Code that passed `endpoint.path` on as the request path must also pass `endpoint.query`.
- **BREAKING**: `HttpClientRequest` has a new `query: Option<&str>` field, sent after `path` as `?query`. Set it to `None` when the query is already part of `path`, which is still accepted.

### Fixed

- The client no longer panics on a response without any header lines, such as `HTTP/1.1 204 No Content\r\n\r\n`: the headers slice was computed with its start past its end. Such responses now parse with an empty header list.
- The server now supports chunked request bodies. A `Transfer-Encoding: chunked` POST used to be treated as complete at the end of its headers, so handlers saw an empty body. The server now reads until the last chunk (and any trailers) and decodes the body in place before parsing the request. A malformed chunk gets `400 Bad Request`, a body that does not fit the request buffer `413`, and a transfer coding other than a single `chunked` (such as `gzip`) `501 Not Implemented`.
- Malformed or contradictory body framing is rejected instead of guessed. A request or response with a `Content-Length` that is not plain decimal digits (`abc`, `+5`, empty), or with repeated `Content-Length` values that disagree, used to have the first value used or the header ignored; the server now answers `400`, and the client returns `InvalidResponse("Invalid Content-Length")` or `InvalidResponse("Conflicting Content-Length")`. Repeated identical values (`5`, `5, 5`) are accepted. A request with both `Transfer-Encoding: chunked` and `Content-Length` is rejected with `400`, since a proxy and the server could disagree on where it ends (request smuggling).
- The client now sends `Content-Length: 0` for a `POST`, `PUT` or `PATCH` without a body. Some servers answer such requests with `411 Length Required`.
- Requests and responses no longer pass CR, LF or other control characters through to the wire, which allowed header and request injection. `HttpClient`, `HttpTlsClient` and the Embassy clients now return `InvalidUrl` for an empty request path, or a path or `Host` value containing spaces or control characters (for example a path built from user input such as `"/x HTTP/1.1\r\nHost: evil\r\n\r\nGET /"`), and `HeaderError` for a header name that is not a valid token or a header value containing control characters other than tab. `HttpResponse::build_bytes()` and `build_head_bytes()` apply the same header checks, so a handler that reflects input into a header such as `Location` can no longer split the response; the server then sends nothing for that connection and `handle_connection()` returns `HeaderError`.
- `parse_endpoint()` no longer mis-parses or silently accepts malformed URLs. A fragment (`#...`) is dropped instead of being sent in the request target. A query on a URL without a path (`http://host?x=1`) used to end up in the host name; it is now parsed as the path `/` with the query `x=1`. `InvalidUrl` is returned for an empty host, a port that is not a number in `1..=65535` (it used to fall back to the default port, so `host:abc` became a host name), bare or unterminated IPv6 hosts, and user information (`user@host`). An empty port (`host:`) means the default port.
- The Embassy-backed clients now send the port in the `Host` header for non-default ports. A request to `http://example.com:8080/` previously sent `Host: example.com`, which breaks virtual-host routing and makes servers build redirects with the wrong port. `HttpClientRequest::host` documents that transport-generic callers should do the same, using `HttpEndpoint::host_header()`.
- Responses without a body are now complete at the end of the headers. Responses to `HEAD` requests, and `1xx`, `204` and `304` responses, often advertise a `Content-Length` or `Transfer-Encoding` for a body that is never sent; the client waited for those bytes and failed with `InvalidResponse("Incomplete response body")` once the connection closed (or stalled until it did).
- Chunked responses are no longer cut short or left hanging. The client decided a chunked response was complete by searching the buffer for `0\r\n\r\n`, which also matches the end of a header block ending in `0` (such as `Age: 0`) and chunk data containing that sequence, so reading stopped early and the body came back empty or truncated. Completion now follows the chunk framing, and trailers after the last chunk are handled. Malformed framing is reported by `dechunk` instead of stalling the read.
- Chunked response decoding no longer panics on a chunk size close to `usize::MAX`: the chunk end offset overflowed (a panic in debug builds, an invalid slice range in release builds), letting a malicious or buggy server crash the client. Such responses now return `InvalidResponse`.

## [0.13.0] - 2026-09-01

### Added

- Added an `embassy` feature, making `embassy-net` and `embassy-time` optional dependencies.
- Added `HttpClient` and `HttpClientRequest` for running the HTTP client over any already-connected `embedded-io-async` stream without Embassy.
- Added `HttpTlsClient` for running TLS over any already-connected `embedded-io-async` stream without Embassy when `tls` is enabled. The client stores a `TlsVerification` policy: `TlsVerification::Verified` checks the certificate chain against a pinned root CA, the hostname, and (with a clock supplying Unix time) the validity period via `embedded-tls`'s no_std `rustpki` backend; `TlsVerification::Unverified` skips certificate checks and remains vulnerable to man-in-the-middle attacks.
- Added `HttpServer` and `handle_http_connection()` for serving a single HTTP connection over any `embedded-io-async` stream without Embassy.
- Added optional `smoltcp` feature with `SmolTcpStream`, an `embedded-io-async` adapter for `smoltcp` TCP sockets.
- Added `TimeoutDuration`, a transport-neutral duration type for `HttpClientOptions`.
- Added `HttpClient::request_with_retry_delay()` and `HttpTlsClient::request_with_retry_delay()`, which sleep between read retries via a caller-provided delay; the Embassy-backed clients use this to honor `HttpClientOptions::retry_delay`.
- Added `HttpEndpoint` and `parse_endpoint()` to the crate root re-exports.
- Added CI coverage for `--no-default-features` and `--no-default-features --features tls`.
- Documented the TLS verification guarantees of each `TlsVerification` policy and the man-in-the-middle risk of unverified HTTPS.

### Changed

- **BREAKING**: `HttpClientOptions` now uses `TimeoutDuration` instead of `embassy_time::Duration`.
- **BREAKING**: Transport error variants no longer expose Embassy error types directly; `DnsError`, `ConnectionError`, and `TcpError` are now transport-neutral variants. Note that stream IO errors on the TLS path are now reported as `TcpError` (previously `TlsError`); TLS handshake errors remain `TlsError`.
- **BREAKING**: The `embassy` feature is no longer enabled by default. Enable `features = ["embassy"]` to use Embassy-backed adapters.
- **BREAKING**: Transport-generic client/server APIs dropped the `Io` infix: `HttpClient`, `HttpClientRequest`, `HttpTlsClient`, `HttpServer`, `DefaultHttpClient`, `DefaultHttpServer`, etc.
- **BREAKING**: Embassy-backed client/server root exports are now explicitly named `DefaultEmbassyHttpClient`, `EmbassyHttpClient`, `SmallEmbassyHttpClient`, `DefaultEmbassyHttpServer`, `EmbassyHttpServer`, and `SmallEmbassyHttpServer`.
- **BREAKING**: `ServerTimeouts` now uses `TimeoutDuration` values instead of raw `u64` seconds and is available only with the `embassy` feature; `HttpServer::with_buffer_sizes()` was renamed to `HttpServer::new()` (buffer sizes are type parameters).
- `SmallHttpClient` now uses a 512-byte request buffer (previously identical to the 1024-byte `DefaultHttpClient`).
- Embassy-backed clients now sleep for `HttpClientOptions::retry_delay` between read retries.
- Embassy-backed client and server modules are now gated behind the `embassy` feature.
- Internal layout is now hexagonal: `client/` and `server/` contain transport-neutral IO adapters plus Embassy adapters.

### Fixed

- Incomplete responses no longer silently return truncated bodies: the client returns `InvalidResponse` when a length-delimited body is cut short by connection close, and `BufferOverflow` when the response exceeds the caller's buffer.
- Incomplete requests no longer silently reach the handler: the server returns `InvalidResponse` on premature close and `BufferOverflow` when a request exceeds the request buffer.
- Fixed integer overflow in the server's request completion check for absurd `Content-Length` values.
- Chunked responses with intermediate transfer codings (e.g. `Transfer-Encoding: gzip, chunked`) are now dechunked correctly.
- TLS handshake seed now uses the low 32 bits of the system tick counter and never collapses to the all-zero XORShift state.
- Removed a redundant per-response copy of the parsed header list.

## [0.12.1] - 2026-06-30

### Added

- Added zero-copy request target and query helpers on `HttpRequest`:
  - `target()` returns the raw request target.
  - `route_path()` returns the path without the query string.
  - `query_string()` returns the raw query string.
  - `query_pairs()` iterates raw query parameter pairs.
  - `query()`, `query_first()`, `query_last()`, `query_all()`, `query_indexed()`, and `query_param()` provide URLSearchParams-style access while preserving duplicate keys.
- Added request convenience helpers: `header()`, `body_str()`, `content_type()`, and `content_length()`.
- Added `percent_decode()` for explicit, no-allocation query component decoding into a caller-provided buffer.
- Re-exported `QueryPair`, `QueryPairs`, `QueryValues`, and `percent_decode` from the crate root.

### Notes

- Query helpers return raw, undecoded values by default.
- Duplicate query keys are preserved.
- Bracket-style query names such as `f[0]` are treated literally.

## [0.12.0] - 2026-06-15

### Changed

- `HttpHandler::handle_request` now takes `&mut self` instead of `&self`.

## [0.11.9] - 2026-06-04

### Added

- `HttpResponseBuilder` with fluent API for flexible response construction:
  - `status(code)` — set HTTP status
  - `header(name, value)` / `headers(&[hdr])` — add custom headers
  - `content_type(ct)` — set Content-Type header
  - `json(body)` — sets `Content-Type: application/json` + text body
  - `problem_json(body)` — sets `Content-Type: application/problem+json` + text body (RFC 7807)
  - `text(body)` / `binary(data)` / `empty_body()` — set body
  - `build()` → `Result<HttpResponse>` — builds with header dedup (last value wins)
- `mime_types::PROBLEM_JSON` constant for RFC 7807 Problem Details for HTTP APIs

### Removed

- **YANKED**: v0.11.8 convenience constructors (`json()`, `text()`, `not_found()`, `bad_request()`) - poorly designed, too opinionated
  - `json()` was hardcoded to 200 OK (invalid for error responses)
  - `bad_request()` was hardcoded to `text/plain` (invalid for RFC 7807 JSON errors)
  - Not flexible enough for real-world API patterns

### Migration Guide (from 0.11.8)

**Before (removed):**
```rust,ignore
HttpResponse::json(r#"{"status":"ok"}"#)          // hardcoded 200
HttpResponse::bad_request("missing parameter")    // hardcoded text/plain
```

**After (builder):**
```rust,ignore
// JSON success
HttpResponseBuilder::new().json(r#"{"status":"ok"}"#)?.build()?

// JSON error (RFC 7807)
HttpResponseBuilder::new()
    .status(StatusCode::BadRequest)
    .problem_json(r#"{"type":"https://example.com/probs/invalid","title":"Invalid parameter"}"#)?
    .build()?

// Text with custom Content-Type
HttpResponseBuilder::new()
    .status(StatusCode::NotFound)
    .content_type(mime_types::TEXT)?
    .text("Not Found")
    .build()?
```

## [0.11.8] - YANKED (2026-06-03)

**Yanked due to poorly designed convenience constructors.** See v0.11.9 for replacement builder pattern.

## [0.11.7] - 2026-06-01

### Added

- SSE-friendly header helpers for `text/event-stream`, `Cache-Control`, and `Connection`.
- `HttpResponse::build_head_bytes` for header-only responses, which makes streaming endpoints easier to wire up in embedded handlers.

### Changed

- Updated the README to call out the HTTP server path for embedded APIs such as Puck.


## [0.11.6] - 2026-05-27

### Added

- Strict clippy configuration following nulid pattern: forbids `unwrap`/`expect`/`panic`/`todo`/`unimplemented` in production code (tests allow these via `.clippy.toml`), enables pedantic and nursery lints at warn level.

### Changed

- Replaced `rand_chacha` dependency with tiny XORShift32 PRNG for TLS seeding (keeps `rand_core 0.6` required by embedded-tls, removes rand 0.10.x conflict).
- Added `const` to all functions that can be made `const` (`ResponseBody::as_bytes`, `ResponseBody::is_empty`, `ResponseBody::len`, `StatusCode::as_u16`, `StatusCode::text`, `ServerTimeouts::new`, `HttpServer::with_timeouts`, `XorShift32Rng::new`).
- Use `Self` instead of type names within impls (`StatusCode`, `Error`, `HttpMethod`, `ResponseBody`).
- Simplified `if let/else` patterns to `map_or`/`map_or_else` for better code clarity.
- Added `Eq` derives to enums with `PartialEq` (`HttpMethod`, `InvalidHttpMethod`).
- Replace `unwrap()` with safe array indexing for `timeseed()` in TLS seeding.
- Used `#[expect(clippy::future_not_send)]` instead of `#[allow(...)]` for embedded async functions (documents intent and alerts if futures become `Send`).

### Fixed

- All 197 clippy warnings (pedantic/nursery lints) fixed without `#[allow(...)]` directives (except expected `future_not_send` for embedded no_std).

## [0.11.5] - 2026-05-21

### Added

- `Error::BufferOverflow` variant for request/response buffer overflow errors.
- Server read loop: accumulates data across multiple TCP segments until headers and `Content-Length` body are fully received.
- IPv4/IPv6 dual-stack DNS resolution: tries A (IPv4) first, falls back to AAAA (IPv6).

### Fixed

- Client read loop now returns error on retry exhaustion instead of parsing partial/truncated data.
- Empty URL path (e.g. `http://example.com`) now defaults to `"/"` instead of producing an invalid request line.
- Handler timeout now returns `408 Request Timeout` instead of `400 Bad Request`.
- Duplicate `Content-Length` header no longer emitted when caller already provides one in `build_bytes`.
- `build_bytes` now returns `Result` and reports `BufferOverflow` instead of silently truncating.
- `ResponseBody::Empty.as_str()` now returns `None` instead of `Some("")`.
- Client handles missing or invalid `Content-Length` (e.g. Traefik stripping headers) by reading until connection close.

### Changed

- Enabled both `proto-ipv4` and `proto-ipv6` in `embassy-net` features for dual-stack support.
- Extracted `resolve_host` helper to deduplicate DNS resolution between HTTP and HTTPS paths.
- Replaced all hardcoded HTTP strings in production code with constants from `protocol.rs` and `header.rs` (`CONTENT_TYPE`, `CONTENT_LENGTH`, `HEADER_SEPARATOR`, `HTTP_VERSION_PREFIX`, `mime_types::*`).
- Server error responses extracted into `text_error_response` helper (DRY).
- `handle_connection` changed from `&mut self` to `&self`.
- `try_push!` macro now returns `Error::BufferOverflow` instead of `Error::InvalidResponse`.
- Removed dead `url_parts` vec and `MAX_URL_PARTS` constant from URL parsing.
- Socket timeout reset after accept to avoid racing with read timeout.
- Removed blanket `#[allow(dead_code)]` on `StatusCode` impl.

## [0.11.4] - 2026-05-18

### Added

- Support for `Transfer-Encoding: chunked` responses. Chunked bodies are decoded in-place before parsing ([#29](https://github.com/rttfd/nanofish/issues/29)).
- New `protocol` module with shared HTTP constants (`CRLF`, `DOUBLE_CRLF`, `MAX_HEADERS`, `HTTP_VERSION`, port defaults) and utilities (`find_double_crlf`, `find_crlf`, `find_header_value`).

### Fixed

- Removed unsafe dangling pointer usage from README examples ([#28](https://github.com/rttfd/nanofish/issues/28)).

### Changed

- Refactored codebase to eliminate magic numbers and duplicated constants (SOLID/DRY).
- Consolidated `MAX_HEADERS` into a single definition in `protocol` module (was defined separately in `client.rs`, `request.rs`, and hardcoded in `response.rs`).
- Replaced scattered `windows(4).position(...)` patterns with shared `protocol::find_double_crlf` and `protocol::find_crlf` utilities.
- Bumped `defmt` from `1.0.1` to `1.1.0`.
- Bumped `heapless` from `0.9.2` to `0.9.3`.

## [0.11.3] - 2026-04-22

### Fixed

- Fixed requests failing with `Error::InvalidResponse("Invalid HTTP response encoding")` on binary responses (e.g., PNG images). The HTTP response parser now only requires headers to be valid UTF-8, not the entire response body ([#26](https://github.com/rttfd/nanofish/issues/26)).

### Changed

- Bumped `embassy-net` from `0.9.0` to `0.9.1`.

## [0.11.2] - 2026-03-30

### Fixed

- Fixed `log` feature not enabling `embassy-net/log`, causing compile failures when using the `log` feature.
- Removed `defmt` from `embassy-net` default features (was accidentally always enabled).

## [0.11.1] - 2026-03-27

### Changed

- Bumped `embassy-net` from `0.8.0` to `0.9.0`.
- Bumped `embassy-time` from `0.5.0` to `0.5.1`.
- Bumped `heapless` from `0.9.1` to `0.9.2`.
- Bumped `futures-lite` from `2.0` to `2.6`.
- Bumped MSRV to `1.91` (required by `heapless` 0.9.2 and `smoltcp` 0.13.0).
- Updated `Makefile` to use `rust-version` from `Cargo.toml` and align with CI workflows.
- Updated GitHub Actions workflows to use `actions/checkout@v5` and `actions/cache@v5` (Node.js 24 compatible).

## [0.11.0] - 2026-03-14

### Changed

- **BREAKING**: `HttpHandler::handle_request` now takes `&self` instead of `&mut self`. Handlers that need mutation can use interior mutability (e.g., `RefCell`, atomics).
- Added `Makefile` with targets for `fmt`, `fmt-check`, `clippy`, `clippy-all`, `test`, `test-all`, `ci`, and `publish`.
- CI workflows now use `make` commands.

## [0.10.0] - 2026-03-10

### Added

- `defmt` feature flag — `defmt` is now optional instead of always enabled.
- `log` feature flag — alternative logging backend using the `log` crate.
- Unified logging macros (`trace!`, `debug!`, `info!`, `warn!`, `error!`) that dispatch to `defmt`, `log`, or no-op depending on the enabled feature.
- Compile-time guard preventing both `defmt` and `log` features from being enabled simultaneously.
- `socket.flush()` call after writing responses in the HTTP server.
- CI workflow matrix testing all valid feature combinations.

### Changed

- **BREAKING**: `defmt` is no longer a hard dependency — users must opt in via `features = ["defmt"]`.
- **BREAKING**: Bumped `embassy-net` from `0.7.1` to `0.8.0`.
- **BREAKING**: Bumped `embedded-io-async` from `0.6.1` to `0.7.0`.
- **BREAKING**: Bumped `embedded-tls` from `0.17.0` to `0.18.0` (new `UnsecureProvider` API).
- Server logging now uses the unified logging macros instead of calling `defmt` directly.
- Client logging now uses the unified logging macros instead of calling `defmt` directly.
- CI workflows no longer use `--all-features` (incompatible with mutually exclusive `defmt`/`log` features).

### Removed

- Direct `defmt` dependency from the default build — it is now behind a feature gate.
- Unused `NoVerify` import from the TLS client code.

### Fixed

- Server failed to compile without the `defmt` feature due to bare `defmt::warn!` / `defmt::info!` calls.
- CI workflows failed with `--all-features` due to mutually exclusive `defmt` and `log` features.

## [0.9.1] - 2025-04-17

### Changed

- Updated README.

## [0.9.0] - 2025-04-17

### Added

- HTTP server implementation (`HttpServer`, `DefaultHttpServer`, `SmallHttpServer`).
- `HttpHandler` trait and `SimpleHandler` for handling incoming requests.
- `HttpRequest` type with parsing from raw bytes.
- `ServerTimeouts` configuration.
- `HttpResponse::build_bytes` for constructing raw HTTP response bytes.

## [0.8.0] - 2025-04-16

### Changed

- **BREAKING**: Added const generic parameter `RQ` for HTTP request buffer size.
- **BREAKING**: Added const generics for TCP and TLS buffer sizes (`TCP_RX`, `TCP_TX`, `TLS_READ`, `TLS_WRITE`).
- Introduced `DefaultEmbassyHttpClient` and `SmallEmbassyHttpClient` type aliases.

## [0.7.0] - 2025-04-13

### Changed

- **BREAKING**: `StatusCode` is now more permissive with an `Other(u16)` variant for unknown codes.
- Implemented `StatusCode` on `HttpResponse`.
- Renamed `reason_phrase` to `text` on `StatusCode`.

## [0.6.0] - 2025-04-13

### Added

- `StatusCode` enum with all standard HTTP/1.1 status codes (RFC 2616).
- `From<u16>` and `TryFrom<&str>` implementations for `StatusCode`.

## [0.5.1] - 2025-04-13

### Fixed

- Version metadata fix.

## [0.5.0] - 2025-04-12

### Changed

- **BREAKING**: Zero-copy HTTP client — response body now borrows directly from user-provided buffers.
- Added more tests.

## [0.4.0] - 2025-04-12

### Changed

- **BREAKING**: Improved headers API and response body handling.

## [0.3.0] - 2025-04-12

### Added

- TLS support via the `tls` feature flag and `embedded-tls`.

## [0.2.0] - 2025-04-12

### Changed

- **BREAKING**: Content-Type is now driven by the user instead of being auto-detected.

## [0.1.1] - 2025-04-11

### Changed

- Updated `Cargo.toml` metadata and README.

## [0.1.0] - 2025-04-11

### Added

- Initial release.
- `no_std` async HTTP client built on Embassy networking.
- Support for GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS, TRACE, and CONNECT methods.
- Configurable client options (retries, timeouts, delays).

[Unreleased]: https://github.com/rttfd/nanofish/compare/v0.16.0...HEAD
[0.16.0]: https://github.com/rttfd/nanofish/compare/v0.15.0...v0.16.0
[0.15.0]: https://github.com/rttfd/nanofish/compare/v0.14.2...v0.15.0
[0.14.2]: https://github.com/rttfd/nanofish/compare/v0.14.1...v0.14.2
[0.14.1]: https://github.com/rttfd/nanofish/compare/v0.14.0...v0.14.1
[0.14.0]: https://github.com/rttfd/nanofish/compare/v0.13.0...v0.14.0
[0.13.0]: https://github.com/rttfd/nanofish/compare/v0.12.1...v0.13.0
[0.12.1]: https://github.com/rttfd/nanofish/compare/v0.12.0...v0.12.1
[0.12.0]: https://github.com/rttfd/nanofish/compare/v0.11.9...v0.12.0
[0.11.9]: https://github.com/rttfd/nanofish/compare/v0.11.8...v0.11.9
[0.11.8]: https://github.com/rttfd/nanofish/compare/v0.11.7...v0.11.8
[0.11.7]: https://github.com/rttfd/nanofish/compare/v0.11.6...v0.11.7
[0.11.6]: https://github.com/rttfd/nanofish/compare/v0.11.5...v0.11.6
[0.11.5]: https://github.com/rttfd/nanofish/compare/v0.11.4...v0.11.5
[0.11.4]: https://github.com/rttfd/nanofish/compare/v0.11.3...v0.11.4
[0.11.3]: https://github.com/rttfd/nanofish/compare/v0.11.2...v0.11.3
[0.11.2]: https://github.com/rttfd/nanofish/compare/v0.11.1...v0.11.2
[0.11.1]: https://github.com/rttfd/nanofish/compare/v0.11.0...v0.11.1
[0.11.0]: https://github.com/rttfd/nanofish/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/rttfd/nanofish/compare/v0.9.1...v0.10.0
[0.9.1]: https://github.com/rttfd/nanofish/compare/v0.9.0...v0.9.1
[0.9.0]: https://github.com/rttfd/nanofish/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/rttfd/nanofish/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/rttfd/nanofish/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/rttfd/nanofish/compare/v0.5.1...v0.6.0
[0.5.1]: https://github.com/rttfd/nanofish/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/rttfd/nanofish/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/rttfd/nanofish/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/rttfd/nanofish/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/rttfd/nanofish/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/rttfd/nanofish/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/rttfd/nanofish/releases/tag/v0.1.0
