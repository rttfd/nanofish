# abstarct_socket

`abstarct_socket` provides a minimal async TCP abstraction used by `nanooctopus`.

The crate defines transport-agnostic traits and ships adapter modules for Embassy and Tokio so higher-level HTTP/WebSocket logic can run on both embedded and host environments.

## What This Crate Covers

- Unified socket metadata via `SocketInfo` (`local_endpoint`, `remote_endpoint`, `state`).
- Connection lifecycle via `SocketClose`, `AbstractSocketListener`, and `AbstarctSocketConnector`.
- Async stream I/O via re-exported `embedded_io_async` traits (`SocketRead`, `SocketWrite`, `SocketReadReady`, `SocketWriteReady`).
- Readiness waiting via `SocketWaitReadReady` and `SocketWaitWriteReady`.
- Optional closure-based zero-copy style hooks via `SocketReadWith` and `SocketWriteWith`.

## Core Trait Model

- `SocketStream`: combines read/write + readiness traits.
- `AbstractSocket`: `SocketStream + SocketInfo + SocketClose`.
- `ExtendedSocket`: `AbstractSocket + SocketReadWith + SocketWriteWith`.
- `AbstractSocketListener`: async accept interface producing implementation-specific sockets.
- `AbstarctSocketConnector`: async outbound connection interface.

The design keeps the abstraction close to native TCP semantics instead of masking platform behavior behind a large custom API.

## Stream Utilities

`stream_search::StreamSearch` adds parser-oriented helpers on top of `SocketReadWith`:

- `seek_until*`: collect bytes into a `prefix_arena::PrefixArena` buffer until a boundary is found.
- `skip*`: consume bytes without storing them.

Errors are represented by `StreamReadError<T>`:

- `SocketReadError(T)` for underlying transport failures.
- `ReadBufferOverflow` when the arena-backed output buffer is exhausted.

`find_sequence::FindSequence` supports sequence detection across chunk boundaries and is used by `StreamSearch` sequence-based operations.

## Implementations

- `embassy_impl`: adapters for `embassy_net::tcp::TcpSocket`, `TcpReader`, and `TcpWriter`, plus `TcpSocketPool` for queued multi-socket acceptance.
- `tokio_impl`: `TokioTcpListener`, `TokioTcpSocketConnector`, and `TokioSocketWrapper` with read/write-half wrappers.
- `mocks`: mock sockets and streams for deterministic unit and integration tests.

## Feature Flags

- `embassy_impl`: enables Embassy adapters and pool support.
- `tokio_impl`: enables Tokio adapters (requires `std`).
- `mocks`: enables mock transport/testing helpers.
- `std`: enables standard library support for applicable dependencies.
- `proto-ipv6`: enables IPv6 support through Embassy (`embassy-net/proto-ipv6`).
- `defmt` / `log`: choose logging backend via `defmt-or-log` integration.

## Notes

- The crate is `no_std` by default (`std` feature opt-in).
- Crate-level docs are generated from this README via `#![doc = include_str!("../README.md")]`.
