[<img alt="github" src="https://img.shields.io/badge/github-rttfd/nanofish-37a8e0?style=for-the-badge&labelColor=555555&logo=github" height="20">](https://github.com/rttfd/nanofish)
[<img alt="crates.io" src="https://img.shields.io/crates/v/nanofish.svg?style=for-the-badge&color=ff8b94&logo=rust" height="20">](https://crates.io/crates/nanofish)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-nanofish-bedc9c?style=for-the-badge&labelColor=555555&logo=docs.rs" height="20">](https://docs.rs/nanofish)

![Dall-E generated nanofish image](https://raw.githubusercontent.com/rttfd/static/refs/heads/main/nanofish/nanofish.png)

# Nanofish

A lightweight, `no_std` HTTP client for embedded systems built on top of Embassy networking with **true zero-copy response handling**.

Nanofish provides a simple HTTP client implementation that works on constrained environments with no heap allocation, making it suitable for microcontrollers and other embedded systems. It features **zero-copy response handling** where all response data is borrowed directly from user-provided buffers, ensuring maximum memory efficiency.

## Key Features

- **True Zero-Copy Response Handling** - Response data is borrowed directly from user-provided buffers with no copying
- **User-Controlled Memory Management** - You provide the buffer, controlling exactly how much memory is used
- Full `no_std` compatibility with no heap allocations
- Built on Embassy for async networking
- Support for all standard HTTP methods (GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS, TRACE, CONNECT)
- Intelligent response body handling (automatic text/binary detection based on Content-Type)
- Convenient header creation with pre-defined constants and methods
- Automatic handling of common headers
- DNS resolution
- Timeout handling and retries
- Optional TLS/HTTPS support (disabled by default)

## Zero-Copy Architecture

Unlike traditional HTTP clients that copy response data multiple times, Nanofish uses a zero-copy approach:

- **You control the buffer size** - Provide a buffer as large or small as needed for your use case
- **Direct memory references** - Response body contains direct references to data in your buffer
- **No hidden allocations** - All memory usage is explicit and controlled by you
- **Optimal for embedded** - Perfect for memory-constrained environments

## Feature Flags

- `tls`: Enables TLS/HTTPS support via `embedded-tls`. When disabled (default), only HTTP requests are supported and HTTPS requests will return an error.

To use nanofish with HTTP only (default):

```toml
[dependencies]
nanofish = "0.5.1"
```

To use nanofish with TLS/HTTPS support:

```toml
[dependencies]
nanofish = { version = "0.5.1", features = ["tls"] }
```

## Example

```rust
use nanofish::{HttpClient, HttpHeader, ResponseBody, headers, mime_types};
use embassy_net::Stack;

async fn example(stack: &Stack<'_>) -> Result<(), nanofish::Error> {
    // Create an HTTP client with a network stack
    let client = HttpClient::new(stack);
    
    // You control the buffer size - make it as large or small as needed!
    let mut response_buffer = [0u8; 8192]; // 8KB buffer for this example
    
    // Define headers using convenience methods
    let headers = [
        HttpHeader::user_agent("Nanofish/0.5.0"),
        HttpHeader::content_type(mime_types::JSON),
        HttpHeader::authorization("Bearer token123"),
    ];
    
    // Or create headers manually for custom needs
    let custom_headers = [
        HttpHeader { name: "X-Custom-Header", value: "custom-value" },
        HttpHeader::new(headers::ACCEPT, mime_types::JSON),
    ];
    
    // Make a GET request with zero-copy response handling
    let (response, bytes_read) = client.get(
        "http://example.com/api/status", 
        &headers,
        &mut response_buffer  // Your buffer - no hidden allocations!
    ).await?;
    
    println!("Read {} bytes into buffer", bytes_read);
    
    // Check if the request was successful
    if response.is_success() {
        // Get specific headers
        if let Some(content_type) = response.content_type() {
            println!("Content-Type: {}", content_type);
        }
        
        // Handle different body types - all data references your buffer directly!
        match &response.body {
            ResponseBody::Text(text) => {
                // text is a &str referencing data in your response_buffer
                println!("Received text: {}", text);
            }
            ResponseBody::Binary(bytes) => {
                // bytes is a &[u8] referencing data in your response_buffer
                println!("Received {} bytes of binary data", bytes.len());
            }
            ResponseBody::Empty => {
                println!("Empty response body");
            }
        }
    }
    
    Ok(())
}
```

## Zero-Copy Benefits

```rust
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

Nanofish provides convenient ways to work with common HTTP headers:

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

```rust
// The response body is automatically parsed based on content type
match &response.body {
    ResponseBody::Text(text) => {
        // Text content (text/*, application/json, application/xml, etc.)
        println!("Text response: {}", text);
    }
    ResponseBody::Binary(bytes) => {
        // Binary content (images, files, etc.)
        println!("Binary response: {} bytes", bytes.len());
    }
    ResponseBody::Empty => {
        // No response body
        println!("Empty response");
    }
}

// Convenience methods for response analysis
if response.is_success() {
    println!("Request successful! Status: {}", response.status_code);
}

if response.is_client_error() {
    println!("Client error: {}", response.status_code);
}

if response.is_server_error() {
    println!("Server error: {}", response.status_code);
}

// Easy access to common headers
if let Some(content_length) = response.content_length() {
    println!("Content length: {} bytes", content_length);
}
```

## Convenience Methods

Nanofish provides convenience methods for all standard HTTP verbs, all using the same zero-copy approach:

```rust
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

```rust
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

## License

The MIT License (MIT)
Copyright © 2025 rttf.dev

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the “Software”), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED “AS IS”, WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.