[<img alt="github" src="https://img.shields.io/badge/github-rttfd/nanofish-37a8e0?style=for-the-badge&labelColor=555555&logo=github" height="20">](https://github.com/rttfd/nanofish)
[<img alt="crates.io" src="https://img.shields.io/crates/v/nanofish.svg?style=for-the-badge&color=ff8b94&logo=rust" height="20">](https://crates.io/crates/nanofish)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-nanofish-bedc9c?style=for-the-badge&labelColor=555555&logo=docs.rs" height="20">](https://docs.rs/nanofish)

![Dall-E generated nanofish image](https://raw.githubusercontent.com/rttfd/static/refs/heads/main/nanofish/nanofish.png)

# Nanofish

A lightweight, `no_std` HTTP/1.1 client and server for embedded systems, with no heap
allocation.

You provide the buffers, so memory use is fixed at compile time. Responses are parsed in place
and borrow from your buffer. The core works over any `embedded-io-async` stream, and adapters
for Embassy, smoltcp and TLS are opt-in features.

- **Client**: every method, a request builder, redirects, keep-alive sessions, streaming
  downloads and chunked uploads of any size, and HTTPS with certificate verification.
- **Server**: request handlers, streamed request and response bodies, keep-alive,
  path parameters, `ETag`/`304` handling, and static files from any store.
- **Strict parsing**: malformed or oversized requests get `400`, `413`, `431` or `501`,
  and ambiguous framing is rejected instead of guessed.

## Installation

```toml
[dependencies]
nanofish = { version = "0.17", features = ["embassy"] }
```

| Feature   | Adds                                                                    |
|-----------|-------------------------------------------------------------------------|
| *(none)*  | The client and server over any `embedded-io-async` stream               |
| `embassy` | `EmbassyHttpClient`, `EmbassyHttpSession`, `EmbassyHttpServer` (DNS, TCP, timeouts) |
| `smoltcp` | `SmolTcpStream`, which turns a `smoltcp` TCP socket into a stream        |
| `tls`     | `HttpTlsClient`, an HTTPS client built on `embedded-tls`                 |
| `defmt` / `log` | Logging through either crate (mutually exclusive); off by default  |

> **TLS:** only `HttpTlsClient::verified` checks the server certificate. The Embassy client's
> built-in HTTPS and `HttpTlsClient::unverified` do **not**, so they are open to
> man-in-the-middle attacks. The server speaks plain HTTP: put TLS in front of it, or run it
> over a TLS stream you bring.

## Client

```rust
use embedded_io_async::{Read, Write};
use nanofish::{HttpClient, ResponseBody};

async fn status<S: Read + Write>(stream: &mut S) -> Result<(), nanofish::Error> {
    let client = HttpClient::new();
    let mut buffer = [0u8; 2048];
    let (response, _) = client
        .get(stream, "example.com", "/api/status", &[], &mut buffer)
        .await?;
    if let ResponseBody::Text(text) = response.body {
        // `text` borrows from `buffer`: nothing was copied.
        let _ = text;
    }
    Ok(())
}
```

With the `embassy` feature, `EmbassyHttpClient` resolves the URL and opens the connection for
you: `client.get("http://example.com/api/status", &[], &mut buffer)`.

## Server

```rust
use embedded_io_async::{Read, Write};
use nanofish::{
    DefaultHttpServer, Error, HttpHandler, HttpRequest, RequestBody, ServerResponse,
    ServerResponseBuilder, StatusCode,
};

struct Api;

impl HttpHandler for Api {
    async fn handle_request<S: Read + Write>(
        &mut self,
        request: &HttpRequest<'_>,
        body: &mut RequestBody<'_, S>,
    ) -> Result<ServerResponse<'_>, Error> {
        match request.path {
            "/health" => ServerResponseBuilder::new().json(r#"{"status":"ok"}"#)?.build(),
            "/upload" => {
                while let Some(piece) = body.chunk().await? {
                    let _ = piece; // write it to flash, a parser, ...
                }
                ServerResponseBuilder::new().status(StatusCode::NoContent).build()
            }
            _ => ServerResponseBuilder::new().status(StatusCode::NotFound).build(),
        }
    }
}

async fn serve_one<S: Read + Write>(stream: &mut S) -> Result<(), Error> {
    DefaultHttpServer::new().handle_connection(stream, &mut Api).await
}
```

With the `embassy` feature, `EmbassyHttpServer::new(80).serve(stack, Api)` accepts connections
and serves them for good.

## Documentation

The [API documentation](https://docs.rs/nanofish) has the guides with examples:
[`client`](https://docs.rs/nanofish/latest/nanofish/client/) covers streaming, uploads,
sessions, redirects, TLS and buffer sizes, and
[`server`](https://docs.rs/nanofish/latest/nanofish/server/) covers request bodies, streaming
responses, static files, keep-alive and running several connections at once. See
[CHANGELOG.md](CHANGELOG.md) for what changed between versions.

## License

[MIT](https://github.com/rttfd/nanofish/blob/main/LICENSE)
