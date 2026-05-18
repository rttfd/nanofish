# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
- Introduced `DefaultHttpClient` and `SmallHttpClient` type aliases.

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

[Unreleased]: https://github.com/rttfd/nanofish/compare/v0.11.2...HEAD
[0.11.2]: https://github.com/rttfd/nanofish/compare/v0.11.1...v0.11.2
[0.11.1]: https://github.com/rttfd/nanofish/compare/v0.11.0...v0.11.1
[0.11.0]: https://github.com/rttfd/nanofish/compare/v0.10.0...v0.11.0
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