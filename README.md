[<img alt="github" src="https://img.shields.io/badge/github-kdimonych/nanooctopus-37a8e0?style=for-the-badge&labelColor=555555&logo=github" height="20">](https://github.com/kdimonych/nanooctopus)
[<img alt="crates.io" src="https://img.shields.io/crates/v/nanooctopus.svg?style=for-the-badge&color=ff8b94&logo=rust" height="20">](https://crates.io/crates/nanooctopus)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-nanooctopus-bedc9c?style=for-the-badge&labelColor=555555&logo=docs.rs" height="20">](https://docs.rs/nanooctopus)

![Nanooctopus](nanooctopus.jpg)

# Nanooctopus

Nanooctopus is a small async HTTP server crate aimed primarily at `no_std` and embedded targets.
It is designed for environments such as RP2040-class MCUs with WiFi or other networking capabilities, while still being able to run on common desktop targets through a Tokio backend.

The project is server-focused. Its core idea is to keep the HTTP layer platform-agnostic and move platform-specific networking into socket abstractions. That lets the same handler code run in embedded firmware, host-side tests, and local emulators while keeping the architecture close to Embassy-style systems.

## Key Ideas

- **Embedded-first HTTP server** for `no_std` environments
- **Platform-agnostic socket abstraction** for portable handlers and host-side testing
- **Embassy-oriented design** with a concrete Tokio backend for desktop development
- **Worker-based request processing** so multiple server tasks can handle requests concurrently
- **Socket-pool support** for keeping more TCP connections open than there are request workers
- **Streaming response builder** that writes directly to the socket
- **Optional WebSocket upgrade support** behind the `ws` feature
- **No heap requirement on embedded targets** for the core request/response flow

## What Problem It Solves

Modern browsers and network clients often open several connections in advance. On small MCUs, it is usually too expensive to dedicate a full request worker to every open TCP socket.

Nanooctopus separates these concerns:

- the **socket layer** can keep several connections alive
- the **worker layer** can stay relatively small
- the **HTTP handler** remains independent from the underlying runtime

That makes it practical to serve HTTP on constrained devices without forcing the whole system into a one-socket-per-worker design.

## Current Scope

Nanooctopus currently provides:

- an HTTP server
- request parsing
- a staged response builder for streaming replies
- an Embassy socket-pool backend
- a Tokio backend for desktop and host-side runs
- optional WebSocket upgrade handling

The crate does **not** currently document or position itself as an HTTP client library.

## Installation

### Embedded / Embassy

```toml
[dependencies]
nanooctopus = { version = "0.1.0", default-features = false, features = ["embassy_impl"] }
```

### Embedded / Embassy with WebSockets and `defmt`

```toml
[dependencies]
nanooctopus = { version = "0.1.0", default-features = false, features = ["embassy_impl", "ws", "defmt"] }
```

### Desktop / Host Testing with Tokio

```toml
[dependencies]
nanooctopus = { version = "0.1.0", features = ["tokio_impl", "log"] }
```

## Feature Flags

- **`embassy_impl`**: enables the Embassy networking backend
- **`tokio_impl`**: enables the Tokio backend and `std` support
- **`ws`**: enables WebSocket upgrade and frame handling support
- **`defmt`**: enables embedded logging with `defmt` and is intended for Embassy builds
- **`log`**: enables logging for host-side and Tokio-based builds
- **`proto-ipv6`**: forwards IPv6 support to the Embassy/socket stack
- **`std`**: enabled automatically by `tokio_impl`; usually not needed directly

`embassy_impl` and `tokio_impl` are mutually exclusive.

## Architecture

At a high level, Nanooctopus is split into three layers:

### 1. Socket backend

The server depends on socket traits rather than directly on Embassy or Tokio types.
This is what makes the crate portable across embedded and desktop runtimes.

### 2. HTTP server core

The core server:

- accepts a connection from a listener or socket pool
- parses the incoming HTTP request into `HttpRequest`
- invokes your `HttpHandler`
- streams the response back through `HttpResponseBuilder`

### 3. Worker memory

Each worker receives its own scratch buffer through `HttpWorkerMemory`.
That memory is used for request parsing and related temporary data.
You choose the size based on your request shape and device constraints.

## Concurrency Model

Nanooctopus is intended to run with multiple worker tasks.
Each worker calls `HttpServer::serve(...)` with its own `HttpWorkerMemory` and context id.

In the Embassy-oriented setup, a socket pool can keep more sockets available than the number of workers actively processing requests. This is useful for browsers that open several TCP connections before they are all needed.

Typical pattern:

- one socket-pool runner task manages TCP sockets
- several HTTP worker tasks process requests
- each worker has its own parsing memory
- all workers share the same server instance

## Basic Tokio Example

The Tokio example is the fastest way to understand the current API.

```rust,ignore
use nanooctopus::{http_handler, server};

struct HelloWorldHandler;

impl http_handler::HttpHandler for HelloWorldHandler {
    async fn handle_request(
        &mut self,
        _allocator: &mut http_handler::HttpAllocator<'_>,
        _request: &http_handler::HttpRequest<'_>,
        http_socket: &mut impl http_handler::HttpSocketWrite,
        _context_id: usize,
    ) -> Result<http_handler::HttpResponse, http_handler::Error> {
        http_handler::HttpResponseBuilder::new(http_socket)
            .with_status(http_handler::StatusCode::Ok)
            .await?
            .with_header("Content-Type", "text/plain")
            .await?
            .with_body_from_str("Hello, World!")
            .await
    }
}

#[tokio::main(flavor = "local")]
async fn main() {
    let listener = server::socket_listener::TokioTcpListener::new(
        server::SocketEndpoint::new([127, 0, 0, 1].into(), 8080),
    )
    .await;

    let server = server::HttpServer::new(listener, server::ServerTimeouts::default());

    server
        .serve(server::HttpWorkerMemory::<1024>::new(), HelloWorldHandler, 1)
        .await;
}
```

This example exists in:

- [`demos/tokio_hello_world`](demos/tokio_hello_world/README.md)
- <a href="https://github.com/kdimonych/nanooctopus/tree/0.1.0/demos/tokio_hello_world">
  demos/tokio_hello_world
  <img alt="GitHub" src="https://github.githubassets.com/favicons/favicon.svg" height="14">

</a>

## Embassy / RP2040 Example

The Raspberry Pico W example shows the intended embedded deployment model:

- initialize CYW43 WiFi
- bring up Embassy networking
- create a TCP socket pool
- spawn the socket-pool runner
- spawn several HTTP server workers

The example is in:

- [`demos/rasberry_pico_w`](demos/rasberry_pico_w/README.md)
- <a href="https://github.com/kdimonych/nanooctopus/tree/0.1.0/demos/rasberry_pico_w">
  demos/rasberry_pico_w
  <img alt="GitHub" src="https://github.githubassets.com/favicons/favicon.svg" height="14">

</a>

The current example uses these fixed-size resources:

- `SOCKETS = 5`
- `HTTP_SERVER_WORKERS = 2`
- `WORKER_MEMORY = 4096`

That setup demonstrates the main idea of the crate: a device may keep more sockets open than the number of HTTP workers actively serving requests.

## Writing a Handler

Request handling is done by implementing `http_handler::HttpHandler`.

The important method is:

```rust,ignore
async fn handle_request(
    &mut self,
    allocator: &mut http_handler::HttpAllocator<'_>,
    request: &http_handler::HttpRequest<'_>,
    http_socket: &mut impl http_handler::HttpSocketWrite,
    context_id: usize,
) -> Result<http_handler::HttpResponse, http_handler::Error>
```

Handler inputs:

- `allocator`: scratch allocator for request-scoped temporary data
- `request`: parsed HTTP request
- `http_socket`: response sink used by `HttpResponseBuilder`
- `context_id`: worker id, useful for diagnostics and per-worker behavior

The parsed request currently exposes:

- `method`
- `path`
- `version`
- `headers`
- `body`

With the `ws` feature enabled, WebSocket upgrade information is also recognized and routed through `handle_websocket_connection`.

## Building Responses

Responses are streamed in stages. A typical flow is:

```rust,ignore
http_handler::HttpResponseBuilder::new(http_socket)
    .with_status(http_handler::StatusCode::Ok)
    .await?
    .with_header("Content-Type", "text/plain; charset=utf-8")
    .await?
    .with_body_from_str("hello")
    .await
```

The response builder supports:

- status line construction
- incremental header writing
- fixed-size body writing from `&str` or `&[u8]`
- chunked transfer encoding
- convenience helpers for plain text, HTML, compressed pages, and preflight responses

## Timeouts

The server exposes:

```rust,ignore
let timeouts = server::ServerTimeouts::new(read_timeout_secs, handler_timeout_secs);
```

Current defaults are:

- read timeout: `30s`
- handler timeout: `60s`

## WebSocket Support

When the `ws` feature is enabled, Nanooctopus can detect WebSocket upgrade requests and hand the connection to:

```rust,ignore
handle_websocket_connection(...)
```

If you do not implement that method, incoming WebSocket connections are closed by default.

## Limitations

- The library is currently centered on the HTTP server.
- Server-side TLS termination is not provided by Nanooctopus itself.
- On embedded targets, you are responsible for sizing socket buffers, worker count, and worker memory according to your traffic profile.
- `embassy_impl` and `tokio_impl` cannot be enabled together.

## Demos

- `demos/rasberry_pico_w`: Embassy + CYW43 + RP2040 + socket pool + multiple workers
- `demos/tokio_hello_world`: minimal host-side Tokio server

## Development Notes

Nanooctopus is structured to make embedded development less painful:

- the Tokio backend helps run handlers on a normal OS during development
- the platform abstraction helps with host-side tests and emulators
- the Embassy-oriented architecture stays close to the intended MCU deployment model

If you want to understand the current project state, start with the demos rather than older historical documentation.

## License

[MIT](LICENSE)
