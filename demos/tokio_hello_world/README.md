# tokio_hello_world

The simplest possible HTTP server built with **nanooctopus** and a **Tokio** backend.\
It binds to `127.0.0.1:8080` and replies to every request with:

```http
HTTP/1.1 200 OK
Content-Type: text/plain

Hello, World!
```

______________________________________________________________________

## What this example demonstrates

| Concept                                        | Where                      |
| ---------------------------------------------- | -------------------------- |
| Implementing `HttpHandler`                     | `HelloWorldHandler`        |
| Streaming response with `HttpResponseBuilder`  | `handle_request`           |
| Binding a TCP listener with `TokioTcpListener` | `main`                     |
| Running the server with `HttpServer::serve`    | `main`                     |
| Allocating per-request scratch memory          | `HttpWorkerMemory::<1024>` |

______________________________________________________________________

## Key components

### `HelloWorldHandler`

Implements the `HttpHandler` trait.\
`handle_request` is called once per incoming HTTP request.\
The response is **streamed directly** to the socket in three stages using the
type-state builder `HttpResponseBuilder`:

```text
status line  →  headers  →  body
```

No heap allocation is needed; the builder writes each part to the TCP socket as
it is constructed.

### `HttpWorkerMemory<1024>`

A fixed-size scratch buffer (`1024` bytes) used internally by the server to
parse incoming HTTP headers. Increase this value if you expect unusually long
headers.

### `ServerTimeouts::default()`

| Timeout           | Default |
| ----------------- | ------- |
| `accept_timeout`  | 10 s    |
| `read_timeout`    | 30 s    |
| `handler_timeout` | 60 s    |

Pass a custom `ServerTimeouts` struct to `HttpServer::new` if you need
different values.

### Single-threaded runtime

The example uses `#[tokio::main(flavor = "local")]` together with
`tokio::task::spawn_local`. This keeps everything on one OS thread, which is
the typical configuration for an embedded-style server and avoids the overhead
of a multi-threaded scheduler.

______________________________________________________________________

## Building and running

The example is a self-contained workspace (it has its own `[workspace]` entry
in `Cargo.toml`), so all `cargo` commands must be run from the
`demos/tokio_hello_world/` directory.

```sh
cd demos/tokio_hello_world
```

### Build only

```sh
cargo build
```

The compiled binary is placed at `target/debug/tokio_hello_world`.

### Build in release mode

```sh
cargo build --release
```

Binary: `target/release/tokio_hello_world`.

### Run (debug build)

```sh
cargo run
```

### Run (release build)

```sh
cargo run --release
```

Then in another terminal:

```sh
curl http://127.0.0.1:8080/
# Hello, World!
```

Or with verbose HTTP output:

```sh
curl -v http://127.0.0.1:8080/
```

### Check (no binary produced, fastest feedback)

```sh
cargo check
```

______________________________________________________________________

## Dependencies

| Crate                                         | Purpose                           |
| --------------------------------------------- | --------------------------------- |
| `nanooctopus` (features: `tokio_impl`, `log`) | HTTP server library               |
| `tokio` (features: `full`)                    | Async runtime                     |
| `log` + `env_logger`                          | Structured logging via `RUST_LOG` |

Set `RUST_LOG` to control log verbosity. The variable is read at startup by
`env_logger` — it does **not** require a recompile.

| Value                       | What you see                                       |
| --------------------------- | -------------------------------------------------- |
| `RUST_LOG=error`            | Only errors                                        |
| `RUST_LOG=info` *(default)* | Incoming requests and server lifecycle events      |
| `RUST_LOG=debug`            | Internal parser state, socket events, and timeouts |
| `RUST_LOG=trace`            | All of the above plus low-level byte I/O           |

### macOS / Linux

```sh
RUST_LOG=debug cargo run
```

### Windows (PowerShell)

```powershell
$env:RUST_LOG="debug"; cargo run
```

### Windows (cmd)

```cmd
set RUST_LOG=debug && cargo run
```

To restrict verbose output to nanooctopus only (leaving other crates quiet):

```sh
RUST_LOG=nanooctopus=debug cargo run
```
