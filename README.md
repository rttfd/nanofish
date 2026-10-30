[<img alt="github" src="https://img.shields.io/badge/github-rttfd/nanofish-37a8e0?style=for-the-badge&labelColor=555555&logo=github" height="20">](https://github.com/rttfd/nanofish)
[<img alt="crates.io" src="https://img.shields.io/crates/v/nanofish.svg?style=for-the-badge&color=ff8b94&logo=rust" height="20">](https://crates.io/crates/nanofish)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-nanofish-bedc9c?style=for-the-badge&labelColor=555555&logo=docs.rs" height="20">](https://docs.rs/nanofish)

![Dall-E generated nanofish image](https://raw.githubusercontent.com/rttfd/static/refs/heads/main/nanofish/nanofish.png)

# Nanofish

A lightweight, `no_std` HTTP client and server for embedded systems with optional Embassy networking and zero-copy response handling.

Nanofish is designed for embedded systems with limited memory. It provides a simple HTTP client and server that works without heap allocation, making it suitable for microcontrollers and `IoT` devices. The library uses zero-copy response handling where response data is borrowed directly from user-provided buffers, keeping memory usage predictable and efficient.

## Key Features

- **Zero-Copy Response Handling** - Response data is borrowed directly from user-provided buffers with no copying
- **User-Controlled Memory** - You provide the buffer and control exactly how much memory is used
- **Configurable Buffer Sizes** - Compile-time buffer size configuration using const generics for optimal memory usage
- **No Standard Library** - Full `no_std` compatibility with no heap allocations
- **Optional Embassy Integration** - Async client/server integration built on Embassy networking; core HTTP types build without Embassy via `default-features = false`
- **Optional smoltcp Adapter** - Wrap `smoltcp` TCP sockets as `embedded-io-async` streams for the generic client/server APIs
- **Complete HTTP Support** - All standard HTTP methods (GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS, TRACE, CONNECT)
- **HTTP Server** - Built-in async server with customizable timeouts and request handling
- **Smart Response Parsing** - Automatic text/binary detection based on Content-Type headers
- **Easy Header Management** - Pre-defined constants and helper methods for common headers
- **SSE-Friendly Headers** - `text/event-stream`, `Cache-Control`, and `Connection` helpers for streaming endpoints
- **Optional TLS Support** - HTTPS client support with embedded-tls when enabled (server is HTTP-only)
- **Optional Logging** - Choose between `defmt` or `log` for diagnostics, or disable both for zero overhead
- **Timeout & Retry Support** - Built-in handling for network issues, plus optional overall request deadlines
- **Streaming Bodies** - Download large responses through a small buffer into a `BodySink`, and send large or incremental responses with a `ResponseWriter`
- **Request Builder & Encoders** - `RequestBuilder`, JSON/form helpers, percent-encoding, Base64 and `Authorization` header builders, with no allocation
- **Keep-Alive & Redirects** - Reusable connections (`EmbassyHttpSession`, `HttpClient::exchange`, server keep-alive) and opt-in, bounded redirect following
- **Server Helpers** - Path parameters (`/led/:id`), `ETag`/`304` handling, chunked request bodies and proper error responses
- **Proxy Compatible** - Works behind reverse proxies (e.g. Traefik) that may strip or modify `Content-Length` headers
- **DNS Resolution** - Automatic hostname resolution with IPv4/IPv6 dual-stack support

## Installation & Feature Flags

### Core HTTP Support (Default, No Embassy)
```toml
[dependencies]
nanofish = "0.13"
```

### With TLS/HTTPS Support
```toml
[dependencies]
nanofish = { version = "0.13", features = ["tls"] }
```

### With Embassy Client/Server Integration
```toml
[dependencies]
nanofish = { version = "0.13", features = ["embassy"] }
```

### With smoltcp Socket Adapter
```toml
[dependencies]
nanofish = { version = "0.13", features = ["smoltcp"] }
```

### With Logging
```toml
# Using defmt (common in embedded/probe-based workflows)
[dependencies]
nanofish = { version = "0.13", features = ["defmt"] }

# Using the log crate (common in std or defmt-incompatible environments)
[dependencies]
nanofish = { version = "0.13", features = ["log"] }
```

> **Note:** The `defmt` and `log` features are **mutually exclusive**. Enabling both will produce a compile-time error. If neither is enabled, all logging calls are compiled away to no-ops.

The default build includes the transport-neutral HTTP types, parsing, response builders, handlers, headers, methods, status codes, options, and generic `embedded-io-async` client/server helpers without pulling in `embassy-net` or `embassy-time`.

### Available Features
- **`embassy`** - Enables the Embassy-backed async client and server integration. Disabled by default.
- **`smoltcp`** - Enables `SmolTcpStream`, an `embedded-io-async` adapter for `smoltcp` TCP sockets.
- **`tls`** - Enables HTTPS/TLS support via `embedded-tls`
  - When disabled: Only HTTP requests are supported
  - When enabled: HTTPS client support with TLS 1.2/1.3 (server remains HTTP-only)
- **`defmt`** - Enables logging via the [`defmt`](https://github.com/knurling-rs/defmt) framework (commonly used with probe-rs)
- **`log`** - Enables logging via the [`log`](https://docs.rs/log) crate

Features can be combined freely (except `defmt` + `log`), for example `features = ["smoltcp", "tls", "defmt"]`.

> **TLS security note**: Convenience HTTPS clients (`EmbassyHttpClient` HTTPS paths and
> `HttpTlsClient` built with `TlsVerification::Unverified`) do **not** verify the server
> certificate (`embedded-tls` `UnsecureProvider`) and are vulnerable to man-in-the-middle
> attacks; the Embassy TLS client also derives handshake randomness from the system tick
> counter. For verified TLS over any stream — including an Embassy `TcpSocket` — build an
> `HttpTlsClient` with `HttpTlsClient::verified(rng, &mut CertVerifier::new(root_ca))`
> (requires a cryptographic RNG and, for expiry checks, a clock returning Unix time).

## Zero-Copy Architecture

Unlike traditional HTTP clients that copy response data multiple times, Nanofish uses a zero-copy approach:

**Traditional HTTP Clients:**
```shell
Network → Internal Buffer (copy #1) → Response Struct (copy #2) → User Code (copy #3)
```

**Nanofish Zero-Copy:**
```shell
Network → YOUR Buffer (direct) → Zero-Copy References → User Code (no copies!)
```

### Benefits:
- **Better Performance** - No memory copying overhead
- **Memory Efficient** - Uses only the memory you provide
- **Predictable** - No hidden allocations
- **Embedded-Friendly** - Works well in resource-limited environments

---

# HTTP Client

## Generic IO Client Without Embassy

With `default-features = false`, use `HttpClient` over any already-connected `embedded-io-async` stream. This is the non-Embassy client implementation that lives alongside the default Embassy-backed `DefaultEmbassyHttpClient`. Your platform owns DNS, TCP connection setup, timeouts, and accept loops.

With `default-features = false, features = ["tls"]`, use `HttpTlsClient` / `DefaultHttpTlsClient` over an already-connected TCP-like stream. TLS is not coupled to Embassy; the client stores a `TlsVerification` policy and an RNG. With `TlsVerification::Verified` the server certificate chain, hostname, and validity period are verified against a root CA you pin (DER); with `TlsVerification::Unverified` the certificate is **not** checked (see the TLS security note above).

```rust,ignore
use embedded_tls::{pki::CertVerifier, Aes128GcmSha256, Certificate};
use nanofish::{DefaultHttpTlsClient, HttpClientRequest, HttpMethod};

// Clock used for certificate expiry checks; return Unix time when known.
struct WallClock;
impl embedded_tls::TlsClock for WallClock {
    fn now() -> Option<u64> {
        /* from your RTC / NTP */ None
    }
}

async fn request<S>(
    stream: S,
    root_ca_der: &[u8],
    rng: impl rand_core::CryptoRngCore,
) -> Result<(), nanofish::Error>
where
    S: embedded_io_async::Read + embedded_io_async::Write,
{
    // Verified: chain + hostname + validity against a pinned root CA.
    let mut verifier = CertVerifier::<Aes128GcmSha256, WallClock, 4096>::new(
        Certificate::X509(root_ca_der),
    );
    let mut client = DefaultHttpTlsClient::verified(rng, &mut verifier);
    let mut response_buffer = [0u8; 4096];

    let (_response, _used) = client
        .request(
            stream,
            "example.com",
            HttpClientRequest {
                method: HttpMethod::GET,
                host: "example.com",
                path: "/",
                query: None,
                headers: &[],
                body: None,
            },
            &mut response_buffer,
        )
        .await
    // Unverified instead: `DefaultHttpTlsClient::unverified(rng)` — no certificate checks.
}
```

With `features = ["smoltcp"]`, wrap a connected or accepted `smoltcp` TCP socket with `SmolTcpStream` and pass it to `HttpClient`, `HttpTlsClient`, `HttpServer`, or `handle_http_connection()`. Your application still owns the `smoltcp` interface/device polling and socket lifecycle.

```rust,ignore
use nanofish::{HttpClient, SmolTcpStream};

async fn request(socket: &mut smoltcp::socket::tcp::Socket<'_>) -> Result<(), nanofish::Error> {
    let mut stream = SmolTcpStream::new(socket);
    let client = HttpClient::new();
    let mut response = [0; 1024];

    let (_response, _used) = client
        .get(&mut stream, "example.com", "/", &[], &mut response)
        .await?;

    Ok(())
}
```

```rust,ignore
use nanofish::{HttpClient, HttpClientRequest, HttpMethod};

async fn request_without_embassy<S>(stream: &mut S) -> Result<(), nanofish::Error>
where
    S: embedded_io_async::Read + embedded_io_async::Write,
{
    let client = HttpClient::new();
    let mut response_buffer = [0u8; 4096];

    let (response, bytes_read) = client.request(
        stream,
        HttpClientRequest {
            method: HttpMethod::GET,
            host: "example.com",
            path: "/api/status",
            query: Some("verbose=1"),
            headers: &[],
            body: None,
        },
        &mut response_buffer,
    ).await?;

    Ok(())
}
```

## Quick Start

Here's a simple example showing how to use the Embassy-backed client (`features = ["embassy"]`):

```rust,ignore
use nanofish::{DefaultEmbassyHttpClient, HttpHeader, ResponseBody, headers, mime_types};
use embassy_net::Stack;

async fn example(stack: &Stack<'_>) -> Result<(), nanofish::Error> {
    let client = DefaultEmbassyHttpClient::new(stack);
    let mut response_buffer = [0u8; 8192];
    let headers = [
        HttpHeader::user_agent("Nanofish/0.13"),
        HttpHeader::content_type(mime_types::JSON),
        HttpHeader::authorization("Bearer token123"),
    ];
    let custom_headers = [
        HttpHeader { name: "X-Custom-Header", value: "custom-value" },
        HttpHeader::new(headers::ACCEPT, mime_types::JSON),
    ];
    let (response, bytes_read) = client.get(
        "http://example.com/api/status",
        &headers,
        &mut response_buffer
    ).await?;
    println!("Read {} bytes into buffer", bytes_read);
    Ok(())
}
```

## Zero-Copy Benefits

```rust,ignore
// Traditional approach (copies data):
// 1. Read from network → internal buffer (copy #1)
// 2. Parse response → response struct (copy #2) 
// 3. User gets → copied data (copy #3)

// Nanofish zero-copy approach:
// 1. Read from network → YOUR buffer (direct)
// 2. Parse response → references to YOUR buffer (zero-copy)
// 3. User gets → direct references to YOUR buffer (zero-copy)

let mut small_buffer = [0u8; 1024];    // For small responses
let mut large_buffer = [0u8; 32768];   // For large responses

// Same API, different memory usage - YOU decide!
let (small_response, _) = client.get(url, &headers, &mut small_buffer).await?;
let (large_response, _) = client.get(url, &headers, &mut large_buffer).await?;
```

## Header Convenience Features

Nanofish provides helpful APIs for working with HTTP headers:

### Pre-defined Header Constants

```rust
use nanofish::headers;

// Common header names
let content_type = headers::CONTENT_TYPE;     // "Content-Type"
let authorization = headers::AUTHORIZATION;   // "Authorization"
let user_agent = headers::USER_AGENT;         // "User-Agent"
let accept = headers::ACCEPT;                 // "Accept"
```

### Pre-defined MIME Types

```rust
use nanofish::mime_types;

// Common MIME types
let json = mime_types::JSON;    // "application/json"
let xml = mime_types::XML;      // "application/xml"
let text = mime_types::TEXT;    // "text/plain"
let html = mime_types::HTML;    // "text/html"
```

### Convenience Methods

```rust
use nanofish::{HttpHeader, mime_types};
// Easy creation of common headers
let headers = [
    HttpHeader::content_type(mime_types::JSON),
    HttpHeader::authorization("Bearer your-token"),
    HttpHeader::user_agent("MyApp/1.0"),
    HttpHeader::accept(mime_types::JSON),
    HttpHeader::api_key("your-api-key"),
];
```

## Response Handling

Nanofish automatically determines the appropriate response body type based on the Content-Type header:

```rust,ignore
use nanofish::ResponseBody;
// The response body is automatically parsed based on content type
match &response.body {
    ResponseBody::Text(text) => {
        println!("Text response: {}", text);
    }
    ResponseBody::Binary(bytes) => {
        println!("Binary response: {} bytes", bytes.len());
    }
    ResponseBody::Empty => {
        println!("Empty response");
    }
}

if response.is_success() {
    println!("Request successful! Status: {}", response.status_code);
}
if response.is_client_error() {
    println!("Client error: {}", response.status_code);
}
if response.is_server_error() {
    println!("Server error: {}", response.status_code);
}

// You can also check status directly on the status code:
if response.status_code.is_success() {
    println!("Success!");
}
if let Some(content_length) = response.content_length() {
    println!("Content length: {} bytes", content_length);
}
```

## HTTP Methods Support

Nanofish provides convenience methods for all standard HTTP verbs:

```rust,ignore
// All methods require a buffer and return (HttpResponse, bytes_read)
let mut buffer = [0u8; 4096];

// GET request
let (response, bytes_read) = client.get(
    "http://api.example.com/users", 
    &headers, 
    &mut buffer
).await?;

// POST request with JSON body
let json_body = br#"{"name": "John", "email": "john@example.com"}"#;
let post_headers = [
    HttpHeader::content_type(mime_types::JSON),
    HttpHeader::authorization("Bearer token123"),
];
let (response, bytes_read) = client.post(
    "http://api.example.com/users", 
    &post_headers, 
    json_body,
    &mut buffer
).await?;

// PUT request
let (response, bytes_read) = client.put(
    "http://api.example.com/users/123", 
    &headers, 
    update_data,
    &mut buffer
).await?;

// DELETE request
let (response, bytes_read) = client.delete(
    "http://api.example.com/users/123", 
    &headers,
    &mut buffer
).await?;

// Other HTTP methods
let (response, _) = client.patch("http://api.example.com/users/123", &headers, patch_data, &mut buffer).await?;
let (response, _) = client.head("http://api.example.com/status", &headers, &mut buffer).await?;
let (response, _) = client.options("http://api.example.com", &headers, &mut buffer).await?;
let (response, _) = client.trace("http://api.example.com", &headers, &mut buffer).await?;
let (response, _) = client.connect("http://proxy.example.com", &headers, &mut buffer).await?;
```

All methods return a `Result<(HttpResponse, usize), Error>` where:
- `HttpResponse` contains zero-copy references to data in your buffer
- `usize` is the number of bytes read into your buffer

## Client Memory Configuration

Just like the server, you can choose different client sizes:

```rust,ignore
use nanofish::{DefaultEmbassyHttpClient, SmallEmbassyHttpClient, EmbassyHttpClient};

// Default client (4KB buffers) - good for most use cases
let client = DefaultEmbassyHttpClient::new(stack);

// Small client (1KB buffers) - for memory-constrained devices  
let client = SmallEmbassyHttpClient::new(stack);

// Custom client with your own buffer sizes
type CustomClient<'a> = EmbassyHttpClient<'a, 2048, 2048, 8192, 8192, 2048>;
//                              TCP_RX ↑    ↑ TCP_TX  ↑     ↑ TLS_WRITE ↑ REQUEST
//                                           TLS_READ ↑
let client = CustomClient::new(stack);
```

### Buffer Size Parameters
- **`TCP_RX`**: TCP receive buffer size (default: 4096 bytes)
- **`TCP_TX`**: TCP transmit buffer size (default: 4096 bytes)  
- **`TLS_READ`**: TLS read record buffer size (default: 4096 bytes)
- **`TLS_WRITE`**: TLS write record buffer size (default: 4096 bytes)
- **`RQ`**: HTTP request buffer size for building requests (default: 1024 bytes)

Choose buffer sizes based on your memory constraints and expected payload sizes. The request buffer size determines the maximum size of HTTP requests that can be built, including headers and request line.

## Memory Efficiency Examples

Choose your buffer size based on your needs:

```rust,ignore
// Scenario 1: Memory-constrained device (1KB buffer)
let mut tiny_buffer = [0u8; 1024];
let (response, _) = client.get(url, &headers, &mut tiny_buffer).await?;
// Perfect for small API responses, status checks, etc.

// Scenario 2: Streaming large data (32KB buffer)
let mut large_buffer = [0u8; 32768];
let (response, bytes_read) = client.get(large_url, &headers, &mut large_buffer).await?;
// Handle larger responses, file downloads, etc.

// Scenario 3: Reuse the same buffer for multiple requests
let mut shared_buffer = [0u8; 8192];
for url in urls {
    let (response, _) = client.get(url, &headers, &mut shared_buffer).await?;
    process_response(&response);
    // Buffer is reused for each request - no allocations!
}
```

---

# HTTP Server

Nanofish includes a built-in HTTP server perfect for embedded systems and `IoT` devices. The server is async, lightweight, and has customizable timeouts.

For streaming endpoints such as server-sent events, use the `Content-Type: text/event-stream`, `Cache-Control: no-cache`, and `Connection: keep-alive` helpers, plus the response head builder when you need to send headers before the body stream starts.

> **Important Note**: The server only supports plain HTTP connections, not HTTPS/TLS. While the Nanofish client supports both HTTP and HTTPS, the server implementation is HTTP-only. For secure connections in production, use a reverse proxy (like nginx) or load balancer that handles TLS termination.

### Generic IO Server Without Embassy

With `default-features = false`, use `HttpServer` or `handle_http_connection()` to serve one request/response cycle over any `embedded-io-async` stream. This is the non-Embassy server implementation that lives alongside the default Embassy-backed `DefaultEmbassyHttpServer`. Your platform owns listening, accepting, timeouts, and connection lifecycle.

```rust,ignore
use nanofish::{DefaultHttpServer, SimpleHandler};

async fn serve_one_without_embassy<S>(stream: &mut S) -> Result<(), nanofish::Error>
where
    S: embedded_io_async::Read + embedded_io_async::Write,
{
    let mut handler = SimpleHandler;
    let server = DefaultHttpServer::new();
    server.handle_connection(stream, &mut handler).await
}
```

### Basic Server Usage

This example uses the Embassy-backed server (`features = ["embassy"]`).

```rust,ignore
use nanofish::{DefaultEmbassyHttpServer, HttpHandler, HttpRequest, HttpResponse, ResponseBody, StatusCode};
use embassy_net::Stack;

// Create a simple request handler
struct MyHandler;

impl HttpHandler for MyHandler {
    async fn handle_request(&mut self, request: &HttpRequest<'_>) -> Result<HttpResponse<'_>, nanofish::Error> {
        match request.path {
            "/" => Ok(HttpResponse {
                status_code: StatusCode::Ok,
                headers: Vec::new(),
                body: ResponseBody::Text("<h1>Hello World!</h1>"),
            }),
            "/api/status" => Ok(HttpResponse {
                status_code: StatusCode::Ok,
                headers: Vec::new(),
                body: ResponseBody::Text("{\"status\":\"ok\"}"),
            }),
            _ => Ok(HttpResponse {
                status_code: StatusCode::NotFound,
                headers: Vec::new(),
                body: ResponseBody::Text("Not Found"),
            }),
        }
    }
}

async fn run_server(stack: Stack<'_>) -> Result<(), nanofish::Error> {
    let mut server = DefaultEmbassyHttpServer::new(80);  // Listen on port 80
    let handler = MyHandler;
    
    // This runs forever, handling requests
    server.serve(stack, handler).await;
}
```

### Response Builder

Use `HttpResponseBuilder` to construct responses with a fluent API:

```rust,ignore
use nanofish::{HttpResponseBuilder, HttpHandler, HttpRequest, StatusCode, mime_types};

struct ApiHandler;

impl HttpHandler for ApiHandler {
    async fn handle_request(&mut self, request: &HttpRequest<'_>) -> Result<HttpResponse<'_>, nanofish::Error> {
        match request.path {
            "/api/health" => HttpResponseBuilder::new().json(r#"{"status":"ok"}"#)?.build()?,
            "/"           => HttpResponseBuilder::new()
                                .content_type(mime_types::TEXT)?
                                .text("Hello from nanofish!")
                                .build()?,
            "/bad"        => HttpResponseBuilder::new()
                                .status(StatusCode::BadRequest)
                                .problem_json(r#"{"type":"https://example.com/probs/invalid","title":"Invalid parameter"}"#)?
                                .build()?,
            _             => HttpResponseBuilder::new()
                                .status(StatusCode::NotFound)
                                .content_type(mime_types::TEXT)?
                                .text("Not Found")
                                .build()?,
        }
    }
}
```

| Method | Description |
|--------|-------------|
| `status(code)` | Set HTTP status |
| `content_type(ct)` | Set Content-Type header |
| `json(body)` | Content-Type: application/json + text body |
| `problem_json(body)` | Content-Type: application/problem+json + text body (RFC 7807) |
| `text(body)` | Set text body |
| `binary(data)` | Set binary body |
| `empty_body()` | Set empty body |
| `header(name, value)` | Add custom header |
| `build()` → `Result<HttpResponse>` | Build with header dedup (last wins) |

### Dynamic Responses

```rust,ignore
use nanofish::{HttpHandler, HttpRequest, HttpResponse, ResponseBody, StatusCode, HttpHeader};
use heapless::String;
use core::fmt::Write;

struct DynamicHandler {
    buffer: String<128>,
    counter: u32,
}

impl DynamicHandler {
    fn new() -> Self {
        Self { buffer: String::new(), counter: 0 }
    }
}

impl HttpHandler for DynamicHandler {
    async fn handle_request(&mut self, request: &HttpRequest<'_>) -> Result<HttpResponse<'_>, nanofish::Error> {
        match request.path {
            "/api/counter" => {
                self.counter += 1;
                self.buffer.clear();
                let _ = write!(self.buffer, "{{\"count\":{}}}", self.counter);

                let mut headers = heapless::Vec::new();
                let _ = headers.push(HttpHeader::content_type(nanofish::mime_types::JSON));

                Ok(HttpResponse {
                    status_code: StatusCode::Ok,
                    headers,
                    body: ResponseBody::Text(&self.buffer),
                })
            }
            _ => {
                Ok(HttpResponse {
                    status_code: StatusCode::NotFound,
                    headers: heapless::Vec::new(),
                    body: ResponseBody::Empty,
                })
            }
        }
    }
}
```

### Server Memory Configuration

Just like the client, you can choose different server sizes:

```rust,ignore
use nanofish::{DefaultEmbassyHttpServer, SmallEmbassyHttpServer, EmbassyHttpServer};

// Default server (4KB buffers) - good for most use cases
let server = DefaultEmbassyHttpServer::new(80);

// Small server (1KB buffers) - for memory-constrained devices  
let server = SmallEmbassyHttpServer::new(80);

// Custom server with your own buffer sizes
type MyServer = EmbassyHttpServer<2048, 2048, 1024, 8192>;  // RX, TX, Request, Response buffer sizes
let server = MyServer::new(80);
```

### Server Timeouts

You can customize how long the server waits for different operations:

```rust,ignore
use nanofish::{DefaultEmbassyHttpServer, ServerTimeouts, TimeoutDuration};

// Default timeouts: 10s accept, 30s read, 60s handler, 120s per connection
let server = DefaultEmbassyHttpServer::new(80);

// Custom timeouts
let timeouts = ServerTimeouts::new(
    TimeoutDuration::from_secs(5),   // 5 seconds to accept new connections
    TimeoutDuration::from_secs(15),  // 15 seconds of socket inactivity while
                                     // reading/writing a request
    TimeoutDuration::from_secs(30),  // 30 seconds for your handler to process requests
    TimeoutDuration::from_secs(60),  // 60 seconds in total per connection, however
                                     // slowly the peer trickles data
);
let server = DefaultEmbassyHttpServer::with_timeouts(80, timeouts);
```

### Request Information

Your handler receives detailed information about each request:

```rust,ignore
impl HttpHandler for MyHandler {
    async fn handle_request(&mut self, request: &HttpRequest<'_>) -> Result<HttpResponse<'_>, nanofish::Error> {
        // Check the HTTP method
        match request.method {
            HttpMethod::GET => { /* handle GET */ }
            HttpMethod::POST => { /* handle POST */ }
            _ => { /* handle other methods */ }
        }
        
        // Look at the request path
        println!("Path: {}", request.path);
        
        // Check headers
        for header in &request.headers {
            println!("Header: {}: {}", header.name, header.value);
        }
        
        // Access request body (for POST, PUT, etc.)
        if !request.body.is_empty() {
            println!("Body: {} bytes", request.body.len());
        }
        
        // Return your response...
        Ok(HttpResponse { /* ... */ })
    }
}
```

### Simple Built-in Handler

For quick testing, you can use the built-in `SimpleHandler`:

```rust,ignore
use nanofish::{DefaultEmbassyHttpServer, SimpleHandler};

async fn run_test_server(stack: Stack<'_>) {
    let mut server = DefaultEmbassyHttpServer::new(8080);
    let handler = SimpleHandler;  // Serves "/" and "/health" endpoints
    
    server.serve(stack, handler).await;
}
```

The `SimpleHandler` provides:
- `GET /` → HTML welcome page
- `GET /health` → JSON status response  
- Everything else → 404 Not Found

# Ergonomics and Performance

## Building Requests

`RequestBuilder` collects the method, headers and body; `send` adds the destination. The `encode`
module builds query strings, form bodies and `Authorization` values without allocating.

```rust,ignore
use nanofish::{HttpClient, RequestBuilder};
use nanofish::encode::{QueryBuilder, bearer_auth};

async fn toggle<S>(stream: &mut S) -> Result<(), nanofish::Error>
where
    S: embedded_io_async::Read + embedded_io_async::Write,
{
    let client = HttpClient::new();
    let mut buffer = [0u8; 1024];

    let token = bearer_auth::<64>("s3cret")?;
    let mut query = QueryBuilder::<64>::new();
    query.pair("fade", "1")?.pair("name", "front door")?;
    let path = heapless::format!(128; "/led/3?{}", query.as_str()).map_err(|_| nanofish::Error::BufferOverflow)?;

    let request = RequestBuilder::put().authorization(&token).json(r#"{"on":true}"#);
    let (response, _) = client.send(stream, "example.com", &path, &request, &mut buffer).await?;
    Ok(())
}
```

`post_json` is a shorthand for the common case, and `basic_auth`, `percent_encode` and
`base64_encode` are available as well.

## Following Redirects

Redirects are returned to the caller by default. Set `max_redirects` and the Embassy clients follow
`301`, `302`, `303`, `307` and `308` for you, reusing the response buffer:

```rust,ignore
use nanofish::{DefaultEmbassyHttpClient, HttpClientOptions};

let client = DefaultEmbassyHttpClient::with_options(
    stack,
    HttpClientOptions { max_redirects: 3, ..HttpClientOptions::default() },
);
```

A `POST` answered by `301`, `302` or `303` is repeated as a `GET`; `Authorization` and `Cookie`
headers are dropped when a redirect leaves the original origin; a redirect from `https` to `http`
is refused. `total_timeout` bounds the whole chain.

## Streaming Large Responses

`request_streaming` hands the body to a `BodySink` as it arrives, so the buffer only has to hold the
status line and headers plus one piece of the body. A chunked body is decoded on the fly.

```rust,ignore
use nanofish::{BodySink, Error, HttpResponse};

struct Flash { written: usize }

impl BodySink for Flash {
    async fn head(&mut self, response: &HttpResponse<'_>) -> Result<(), Error> {
        // Refuse anything but a complete image before touching flash.
        if response.status_code != nanofish::StatusCode::Ok {
            return Err(Error::InvalidResponse("unexpected status"));
        }
        Ok(())
    }

    async fn chunk(&mut self, data: &[u8]) -> Result<(), Error> {
        // write `data` at `self.written`, then:
        self.written += data.len();
        Ok(())
    }
}

let mut buffer = [0u8; 1024];
let streamed = client
    .request_streaming(HttpMethod::GET, "http://example.com/firmware.bin", &[], None, &mut buffer, &mut sink)
    .await?;
```

A response that is cut short is an error, never a silent partial download.

## Reusing Connections

`EmbassyHttpClient` opens a connection per request. An `EmbassyHttpSession` connects once and keeps
the socket open while the server does, reconnecting by itself (and repeating safe requests once)
when the server dropped it:

```rust,ignore
use nanofish::{EmbassyHttpSession, HttpClientOptions, HttpMethod};

let mut session = EmbassyHttpSession::<1024>::connect(
    stack, &mut rx, &mut tx, "http://sensor-hub.local", HttpClientOptions::default(),
).await?;
let (response, _) = session.request(HttpMethod::GET, "/api/temperature", &[], None, &mut buffer).await?;
let (response, _) = session.request(HttpMethod::GET, "/api/humidity", &[], None, &mut buffer).await?;
```

Without Embassy, `HttpClient::exchange(stream, request, buffer, keep_alive)` tells you whether the
stream can carry another request.

## Verified HTTPS on Embassy

`EmbassyHttpClient::request` does not verify server certificates. To verify (or pin) them while
still using Embassy for DNS and TCP, give `request_with_tls` an `HttpTlsClient::verified` client:

```rust,ignore
let mut tls = DefaultHttpTlsClient::verified(rng, &mut verifier);
let (response, _) = client
    .request_with_tls(&mut tls, HttpMethod::GET, "https://example.com/", &[], None, &mut buffer)
    .await?;
```

Any `TlsVerifier` works, including your own that compares the certificate with a pinned one.

## Server: Routes, Conditional Requests, Streaming

```rust,ignore
impl HttpHandler for Api {
    async fn handle_request(&mut self, request: &HttpRequest<'_>) -> Result<HttpResponse<'_>, Error> {
        if let Some(params) = request.match_path::<2>("/led/:id") {
            let id = params.get("id");
            // ...
        }
        if request.if_none_match("\"v7\"") {
            return HttpResponseBuilder::new().status(StatusCode::NotModified).etag("\"v7\"")?.build();
        }
        HttpResponseBuilder::new().etag("\"v7\"")?.text("hello").build()
    }
}
```

`HEAD` requests get their headers without the body, and malformed or oversized requests are answered
with `400`, `413`, `431` or `501` instead of a silent close. Chunked request bodies are decoded
before your handler runs.

For responses that are large or produced incrementally, implement `StreamingHandler` and write
through a `ResponseWriter`; run it with `EmbassyHttpServer::serve_streaming`:

```rust,ignore
impl StreamingHandler for Log {
    async fn handle<S: Write, const HEAD_SIZE: usize>(
        &mut self,
        _request: &HttpRequest<'_>,
        response: &mut ResponseWriter<'_, S, HEAD_SIZE>,
    ) -> Result<(), Error> {
        response.send_head(StatusCode::Ok, &[HttpHeader::content_type(mime_types::TEXT)], BodyLength::Unknown).await?;
        for line in self.lines() {
            response.write(line).await?;   // sent as a chunk
        }
        Ok(()) // finished for you
    }
}
```

`EmbassyHttpServer::with_keep_alive(n)` serves up to `n` requests per connection. A connection that
sits idle keeps the server's socket busy until `read_timeout` expires, so lower that timeout when
enabling it, and note that one socket serves one client at a time. To serve several clients at once,
run `serve` in several tasks: each owns its socket and buffers and all accept on the same port.

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for a full list of changes across all versions.

## License

[MIT](license)
