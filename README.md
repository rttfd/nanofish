[<img alt="github" src="https://img.shields.io/badge/github-rttfd/nanofish-37a8e0?style=for-the-badge&labelColor=555555&logo=github" height="20">](https://github.com/rttfd/nanofish)
[<img alt="crates.io" src="https://img.shields.io/crates/v/nanofish.svg?style=for-the-badge&color=ff8b94&logo=rust" height="20">](https://crates.io/crates/nanofish)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-nanofish-bedc9c?style=for-the-badge&labelColor=555555&logo=docs.rs" height="20">](https://docs.rs/nanofish)

![Dall-E generated nanofish image](https://raw.githubusercontent.com/rttfd/static/refs/heads/main/nanofish/nanofish.png)

# Nanofish

A lightweight, `no_std` HTTP client for embedded systems built on Embassy networking with zero-copy response handling.

Nanofish is designed for embedded systems with limited memory. It provides a simple HTTP client that works without heap allocation, making it suitable for microcontrollers and `IoT` devices. The library uses zero-copy response handling where response data is borrowed directly from user-provided buffers, keeping memory usage predictable and efficient.

## Key Features

- **Zero-Copy Response Handling** - Response data is borrowed directly from user-provided buffers with no copying
- **User-Controlled Memory** - You provide the buffer and control exactly how much memory is used
- **Configurable Buffer Sizes** - Compile-time buffer size configuration using const generics for optimal memory usage
- **No Standard Library** - Full `no_std` compatibility with no heap allocations
- **Embassy Integration** - Built on Embassy's async networking
- **Complete HTTP Support** - All standard HTTP methods (GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS, TRACE, CONNECT)
- **Smart Response Parsing** - Automatic text/binary detection based on Content-Type headers
- **Easy Header Management** - Pre-defined constants and helper methods for common headers
- **Optional TLS Support** - HTTPS support with embedded-tls when enabled
- **Timeout & Retry Support** - Built-in handling for network issues
- **DNS Resolution** - Automatic hostname resolution

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

## Installation & Feature Flags

### Basic HTTP Support (Default)
```toml
[dependencies]
nanofish = "0.8.0"
```

### With TLS/HTTPS Support
```toml
[dependencies]
nanofish = { version = "0.8.0", features = ["tls"] }
```

### Available Features
- **`tls`** - Enables HTTPS/TLS support via `embedded-tls`
  - When disabled (default): Only HTTP requests are supported
  - When enabled: Full HTTPS support with TLS 1.2/1.3

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
    HttpHeader::user_agent("Nanofish/0.8.0"),
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

## Buffer Size Configuration

Nanofish uses const generics to allow compile-time configuration of internal buffer sizes for optimal memory usage in different environments:

### Default Configuration
```rust,ignore
use nanofish::DefaultHttpClient;

let client = DefaultHttpClient::new(stack);
```

### Memory-Constrained Environments
```rust,ignore
use nanofish::SmallHttpClient;  

let client = SmallHttpClient::new(stack);
```

### Custom Buffer Sizes
```rust,ignore
use nanofish::HttpClient;

// Custom TCP and TLS buffer sizes
type CustomClient<'a> = HttpClient<'a, 2048, 2048, 8192, 8192>;
//                              TCP_RX ↑    ↑ TCP_TX  ↑     ↑ TLS_WRITE
//                                           TLS_READ ↑
let client = CustomClient::new(stack);
```

### Buffer Size Parameters
- **`TCP_RX`**: TCP receive buffer size (default: 4096 bytes)
- **`TCP_TX`**: TCP transmit buffer size (default: 4096 bytes)  
- **`TLS_READ`**: TLS read record buffer size (default: 4096 bytes)
- **`TLS_WRITE`**: TLS write record buffer size (default: 4096 bytes)

Choose buffer sizes based on your memory constraints and expected payload sizes.

## License

[MIT](license)
