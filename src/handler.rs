use crate::{
    error::Error,
    header::HttpHeader,
    request::HttpRequest,
    response::{HttpResponse, ResponseBody},
    status_code::StatusCode,
};
use heapless::Vec;

/// Trait for handling HTTP requests
#[allow(async_fn_in_trait)]
pub trait HttpHandler {
    /// Handle an incoming HTTP request and return a response
    async fn handle_request(&self, request: &HttpRequest<'_>) -> Result<HttpResponse<'_>, Error>;
}

/// A simple handler that serves basic endpoints for testing
#[derive(Debug)]
pub struct SimpleHandler;

impl HttpHandler for SimpleHandler {
    async fn handle_request(&self, request: &HttpRequest<'_>) -> Result<HttpResponse<'_>, Error> {
        let mut headers = Vec::new();
        match request.path {
            "/" => {
                let _ = headers.push(HttpHeader::new("Content-Type", "text/html"));
                Ok(HttpResponse {
                    status_code: StatusCode::Ok,
                    headers,
                    body: ResponseBody::Text("<h1>Hello from nanofish HTTP server!</h1>"),
                })
            }
            "/health" => {
                let _ = headers.push(HttpHeader::new("Content-Type", "application/json"));
                Ok(HttpResponse {
                    status_code: StatusCode::Ok,
                    headers,
                    body: ResponseBody::Text("{\"status\":\"ok\"}"),
                })
            }
            _ => {
                let _ = headers.push(HttpHeader::new("Content-Type", "text/plain"));
                Ok(HttpResponse {
                    status_code: StatusCode::NotFound,
                    headers,
                    body: ResponseBody::Text("404 Not Found"),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HttpMethod, HttpRequest, StatusCode};
    use heapless::Vec;

    #[test]
    fn test_simple_handler() {
        // Test root path
        let handler = SimpleHandler;
        let request = HttpRequest {
            method: HttpMethod::GET,
            path: "/",
            version: "HTTP/1.1",
            headers: Vec::new(),
            body: b"",
        };

        let response = futures_lite::future::block_on(handler.handle_request(&request)).unwrap();
        assert_eq!(response.status_code, StatusCode::Ok);
        assert_eq!(
            response.body.as_str(),
            Some("<h1>Hello from nanofish HTTP server!</h1>")
        );

        // Test health endpoint
        let handler = SimpleHandler;
        let request = HttpRequest {
            method: HttpMethod::GET,
            path: "/health",
            version: "HTTP/1.1",
            headers: Vec::new(),
            body: b"",
        };

        let response = futures_lite::future::block_on(handler.handle_request(&request)).unwrap();
        assert_eq!(response.status_code, StatusCode::Ok);
        assert_eq!(response.body.as_str(), Some("{\"status\":\"ok\"}"));

        // Test 404 path
        let handler = SimpleHandler;
        let request = HttpRequest {
            method: HttpMethod::GET,
            path: "/nonexistent",
            version: "HTTP/1.1",
            headers: Vec::new(),
            body: b"",
        };

        let response = futures_lite::future::block_on(handler.handle_request(&request)).unwrap();
        assert_eq!(response.status_code, StatusCode::NotFound);
        assert_eq!(response.body.as_str(), Some("404 Not Found"));
    }
}
