[<img alt="github" src="https://img.shields.io/badge/github-rttfd/nanofish-37a8e0?style=for-the-badge&labelColor=555555&logo=github" height="20">](https://github.com/rttfd/nanofish)
[<img alt="crates.io" src="https://img.shields.io/crates/v/nanofish.svg?style=for-the-badge&color=ff8b94&logo=rust" height="20">](https://crates.io/crates/nanofish)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-cetar-bedc9c?style=for-the-badge&labelColor=555555&logo=docs.rs" height="20">](https://docs.rs/nanofish)

![Dall-E generated nanofish image](https://raw.githubusercontent.com/rttfd/static/refs/heads/main/nanofish/nanofish.png)

# Nanofish

A lightweight, `no_std` HTTP client for embedded systems built on top of Embassy networking.

Nanofish provides a simple HTTP client implementation that works on constrained environments with no heap allocation, making it suitable for microcontrollers and other embedded systems. It supports all standard HTTP methods and provides a clean async API for making HTTP requests.

## Features

- Full `no_std` compatibility with no heap allocations
- Built on Embassy for async networking
- Support for all standard HTTP methods (GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS, TRACE, CONNECT)
- Automatic handling of common headers
- DNS resolution
- Timeout handling and retries

## Example

```rust
use nanofish::{HttpClient, HttpHeader};
use embassy_net::Stack;

async fn example(stack: &Stack<'_>) -> Result<(), nanofish::Error> {
    // Create an HTTP client with a network stack
    let client = HttpClient::new(stack);
    // Define custom headers (optional)
    let headers = [
        HttpHeader { name: "User-Agent", value: "Nanofish/0.1.0" },
    ];
    // Make a GET request
    let response = client.get("http://example.com/api/status", &headers).await?;
    // Check the response
    if response.status_code == 200 {
        // Process the response body
        let body = response.body;
    }
    Ok(())
}
```

## Convenience Methods

Nanofish provides convenience methods for all standard HTTP verbs:

- `get(endpoint, headers)`
- `post(endpoint, headers, body)`
- `put(endpoint, headers, body)`
- `delete(endpoint, headers)`
- `patch(endpoint, headers, body)`
- `head(endpoint, headers)`
- `options(endpoint, headers)`
- `trace(endpoint, headers)`
- `connect(endpoint, headers)`

Each method returns a `Result<HttpClientResponse, Error>`.

## License

The MIT License (MIT)
Copyright © 2025 rttf.dev

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the “Software”), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED “AS IS”, WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.