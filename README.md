[<img alt="github" src="https://img.shields.io/badge/github-rttfd/nanofish-37a8e0?style=for-the-badge&labelColor=555555&logo=github" height="20">](https://github.com/rttfd/nanofish)
[<img alt="crates.io" src="https://img.shields.io/crates/v/nanofish.svg?style=for-the-badge&color=ff8b94&logo=rust" height="20">](https://crates.io/crates/nanofish)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-nanofish-bedc9c?style=for-the-badge&labelColor=555555&logo=docs.rs" height="20">](https://docs.rs/nanofish)

![Dall-E generated nanofish image](https://raw.githubusercontent.com/rttfd/static/refs/heads/main/nanofish/nanofish.png)

# Nanofish

A lightweight, `no_std` HTTP client and server for embedded systems built on Embassy networking with zero-copy response handling.

Nanofish is designed for embedded systems with limited memory. It provides a simple HTTP client and server that works without heap allocation, making it suitable for microcontrollers and `IoT` devices. The library uses zero-copy response handling where response data is borrowed directly from user-provided buffers, keeping memory usage predictable and efficient.

## Key Features

- **Zero-Copy Response Handling** - Response data is borrowed directly from user-provided buffers with no copying
- **User-Controlled Memory** - You provide the buffer and control exactly how much memory is used
- **Configurable Buffer Sizes** - Compile-time buffer size configuration using const generics for optimal memory usage
- **No Standard Library** - Full `no_std` compatibility with no heap allocations
- **Embassy Integration** - Built on Embassy's async networking
- **Complete HTTP Support** - All standard HTTP methods (GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS, TRACE, CONNECT)
- **HTTP Server** - Built-in async server with customizable timeouts and request handling
- **Smart Response Parsing** - Automatic text/binary detection based on Content-Type headers
- **Easy Header Management** - Pre-defined constants and helper methods for common headers
- **Optional TLS Support** - HTTPS client support with embedded-tls when enabled (server is HTTP-only)
- **Optional Logging** - Choose between `defmt` or `log` for diagnostics, or disable both for zero overhead
- **Timeout & Retry Support** - Built-in handling for network issues
- **DNS Resolution** - Automatic hostname resolution

## Installation & Feature Flags

### Basic HTTP Support (Default)
```toml
[dependencies]
nanofish = "0.10.0"
```

### With TLS/HTTPS Support
```toml
[dependencies]
nanofish = { version = "0.10.0", features = ["tls"] }
```

### With Logging
```toml
# Using defmt (common in embedded/probe-based workflows)
[dependencies]
nanofish = { version = "0.10.0", features = ["defmt"] }

# Using the log crate (common in std or defmt-incompatible environments)
[dependencies]
nanofish = { version = "0.10.0", features = ["log"] }
```

> **Note:** The `defmt` and `log` features are **mutually exclusive**. Enabling both will produce a compile-time error. If neither is enabled, all logging calls are compiled away to no-ops.

### Available Features
- **`tls`** - Enables HTTPS/TLS support via `embedded-tls`
  - When disabled (default): Only HTTP requests are supported
  - When enabled: Full HTTPS support with TLS 1.2/1.3
- **`defmt`** - Enables logging via the [`defmt`](https://github.com/knurling-rs/defmt) framework (commonly used with probe-rs)
- **`log`** - Enables logging via the [`log`](https://docs.rs/log) crate

Features can be combined freely (except `defmt` + `log`), for example `features = ["tls", "defmt"]`.

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

## Quick Start

Here's a simple example showing how to use Nanofish:

```rust,ignore
use nanofish::{DefaultHttpClient, HttpHeader, ResponseBody, headers, mime_types};
use embassy_net::Stack;

async fn example(stack: &Stack<'_>) -> Result<(), nanofish::Error> {
    ...
    // See crate docs for full async usage example
}

let client = DefaultHttpClient::new(unsafe { core::ptr::NonNull::dangling().as_ref() });
let mut response_buffer = [0u8; 8192];
let headers = [
    HttpHeader::user_agent("Nanofish/0.10.0"),
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
use nanofish::{DefaultHttpClient, SmallHttpClient, HttpClient};

// Default client (4KB buffers) - good for most use cases
let client = DefaultHttpClient::new(stack);

// Small client (1KB buffers) - for memory-constrained devices  
let client = SmallHttpClient::new(stack);

// Custom client with your own buffer sizes
type CustomClient<'a> = HttpClient<'a, 2048, 2048, 8192, 8192, 2048>;
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

> **Important Note**: The server only supports plain HTTP connections, not HTTPS/TLS. While the Nanofish client supports both HTTP and HTTPS, the server implementation is HTTP-only. For secure connections in production, use a reverse proxy (like nginx) or load balancer that handles TLS termination.

### Basic Server Usage

```rust,ignore
use nanofish::{DefaultHttpServer, HttpHandler, HttpRequest, HttpResponse, ResponseBody, StatusCode};
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
    let mut server = DefaultHttpServer::new(80);  // Listen on port 80
    let handler = MyHandler;
    
    // This runs forever, handling requests
    server.serve(stack, handler).await;
}
```

### Server Memory Configuration

Just like the client, you can choose different server sizes:

```rust,ignore
use nanofish::{DefaultHttpServer, SmallHttpServer, HttpServer};

// Default server (4KB buffers) - good for most use cases
let server = DefaultHttpServer::new(80);

// Small server (1KB buffers) - for memory-constrained devices  
let server = SmallHttpServer::new(80);

// Custom server with your own buffer sizes
type MyServer = HttpServer<2048, 2048, 1024, 8192>;  // RX, TX, Request, Response buffer sizes
let server = MyServer::new(80);
```

### Server Timeouts

You can customize how long the server waits for different operations:

```rust,ignore
use nanofish::{DefaultHttpServer, ServerTimeouts};

// Default timeouts: 10s accept, 30s read, 60s handler
let server = DefaultHttpServer::new(80);

// Custom timeouts
let timeouts = ServerTimeouts::new(
    5,   // 5 seconds to accept new connections
    15,  // 15 seconds to read request data
    30   // 30 seconds for your handler to process requests
);
let server = DefaultHttpServer::with_timeouts(80, timeouts);
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
use nanofish::{DefaultHttpServer, SimpleHandler};

async fn run_test_server(stack: Stack<'_>) {
    let mut server = DefaultHttpServer::new(8080);
    let handler = SimpleHandler;  // Serves "/" and "/health" endpoints
    
    server.serve(stack, handler).await;
}
```

The `SimpleHandler` provides:
- `GET /` → HTML welcome page
- `GET /health` → JSON status response  
- Everything else → 404 Not Found

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for a full list of changes across all versions.

## License

[MIT](license)
