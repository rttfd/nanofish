[<img alt="github" src="https://img.shields.io/badge/github-rttfd/nanofish-37a8e0?style=for-the-badge&labelColor=555555&logo=github" height="20">](https://github.com/rttfd/nanofish)
[<img alt="crates.io" src="https://img.shields.io/crates/v/nanofish.svg?style=for-the-badge&color=ff8b94&logo=rust" height="20">](https://crates.io/crates/nanofish)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-nanofish-bedc9c?style=for-the-badge&labelColor=555555&logo=docs.rs" height="20">](https://docs.rs/nanofish)

![Dall-E generated nanofish image](https://raw.githubusercontent.com/rttfd/static/refs/heads/main/nanofish/nanofish.png)

# Nanofish

A lightweight, `no_std` HTTP client for embedded systems built on top of Embassy networking.

Nanofish provides a simple HTTP client implementation that works on constrained environments with no heap allocation, making it suitable for microcontrollers and other embedded systems. It supports all standard HTTP methods and provides a clean async API for making HTTP requests.

## Features

- Full `no_std` compatibility with no heap allocations
- Built on Embassy for async networking
- Support for all standard HTTP methods (GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS, TRACE, CONNECT)
- Intelligent response body handling (automatic text/binary detection)
- Convenient header creation with pre-defined constants and methods
- Automatic handling of common headers
- DNS resolution
- Timeout handling and retries
- Optional TLS/HTTPS support (disabled by default)

## Feature Flags

- `tls`: Enables TLS/HTTPS support via `embedded-tls`. When disabled (default), only HTTP requests are supported and HTTPS requests will return an error.

To use nanofish with HTTP only (default):

```toml
[dependencies]
nanofish = "0.4.0"
```

To use nanofish with TLS/HTTPS support:

```toml
[dependencies]
nanofish = { version = "0.4.0", features = ["tls"] }
```

## Example

```rust
use nanofish::{HttpClient, HttpHeader, ResponseBody, headers, mime_types};
use embassy_net::Stack;

async fn example(stack: &Stack<'_>) -> Result<(), nanofish::Error> {
    // Create an HTTP client with a network stack
    let client = HttpClient::new(stack);
    
    // Define headers using convenience methods
    let headers = [
        HttpHeader::user_agent("Nanofish/0.4.0"),
        HttpHeader::content_type(mime_types::JSON),
        HttpHeader::authorization("Bearer token123"),
    ];
    
    // Or create headers manually for custom needs
    let custom_headers = [
        HttpHeader { name: "X-Custom-Header", value: "custom-value" },
        HttpHeader::new(headers::ACCEPT, mime_types::JSON),
    ];
    
    // Make a GET request
    let response = client.get("http://example.com/api/status", &headers).await?;
    
    // Check if the request was successful
    if response.is_success() {
        // Get specific headers
        if let Some(content_type) = response.content_type() {
            println!("Content-Type: {}", content_type);
        }
        
        // Handle different body types
        match &response.body {
            ResponseBody::Text(text) => {
                println!("Received text: {}", text);
            }
            ResponseBody::Binary(bytes) => {
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

Nanofish provides convenience methods for all standard HTTP verbs:

```rust
// GET request
let response = client.get("http://api.example.com/users", &headers).await?;

// POST request with JSON body
let json_body = b r#"{"name": "John", "email": "john@example.com"}"#;
let post_headers = [
    HttpHeader::content_type(mime_types::JSON),
    HttpHeader::authorization("Bearer token123"),
];
let response = client.post("http://api.example.com/users", &post_headers, json_body).await?;

// PUT request
let response = client.put("http://api.example.com/users/123", &headers, update_data).await?;

// DELETE request
let response = client.delete("http://api.example.com/users/123", &headers).await?;

// Other methods
let response = client.patch("http://api.example.com/users/123", &headers, patch_data).await?;
let response = client.head("http://api.example.com/status", &headers).await?;
let response = client.options("http://api.example.com", &headers).await?;
```

All methods return a `Result<HttpResponse, Error>` and support the same header and response handling features.

## License

The MIT License (MIT)
Copyright © 2025 rttf.dev

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the “Software”), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED “AS IS”, WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.