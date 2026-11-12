use embedded_io_async::{Read, Write};
use heapless::Vec;
use nanofish::{
    error::Error,
    header::{HttpHeader, mime_types},
    request::HttpRequest,
    server::{RequestBody, ServerResponse},
    status_code::StatusCode,
};

/// Trait for handling HTTP requests
///
/// The server reads only the head of a request before it calls the handler:
/// `request` has the method, target and headers, and an empty `body`. The body
/// is read through `body`, as the handler needs it: all at once with
/// [`RequestBody::all`] (decoded in place, no copy), or piece by piece with
/// [`RequestBody::chunk`], so a body of any size, `Content-Length` or chunked,
/// can pass through a small buffer. A handler that does not read the body
/// leaves it to the server, which discards it after the response.
#[expect(
    async_fn_in_trait,
    reason = "no_std embedded executors are single-threaded, so no `Send` bound is wanted"
)]
pub trait HttpHandler {
    /// Handle an incoming HTTP request and return a response
    ///
    /// # Errors
    ///
    /// An error is answered with `500`, except the error of a failed body
    /// read, which is answered like the failure: `400` for bad chunk framing
    /// and `413` for a body that does not fit the request buffer.
    async fn handle_request<S: Read + Write>(
        &mut self,
        request: &HttpRequest<'_>,
        body: &mut RequestBody<'_, S>,
    ) -> Result<ServerResponse<'_>, Error>;
}

/// A simple handler that serves basic endpoints for testing
#[derive(Debug)]
pub struct SimpleHandler;

impl HttpHandler for SimpleHandler {
    async fn handle_request<S: Read + Write>(
        &mut self,
        request: &HttpRequest<'_>,
        _body: &mut RequestBody<'_, S>,
    ) -> Result<ServerResponse<'_>, Error> {
        let mut headers = Vec::new();
        match request.path {
            "/" => {
                let _ = headers.push(HttpHeader::content_type(mime_types::HTML));
                Ok(ServerResponse {
                    status_code: StatusCode::Ok,
                    headers,
                    body: b"<h1>Hello from nanofish HTTP server!</h1>",
                })
            }
            "/health" => {
                let _ = headers.push(HttpHeader::content_type(mime_types::JSON));
                Ok(ServerResponse {
                    status_code: StatusCode::Ok,
                    headers,
                    body: b"{\"status\":\"ok\"}",
                })
            }
            _ => {
                let _ = headers.push(HttpHeader::content_type(mime_types::TEXT));
                Ok(ServerResponse {
                    status_code: StatusCode::NotFound,
                    headers,
                    body: b"404 Not Found",
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use core::convert::Infallible;
    use embedded_io_async::ErrorType;
    use heapless::Vec;
    use nanofish::handler::*;
    use nanofish::server::body::BodyParts;
    use nanofish::{HttpMethod, HttpRequest, StatusCode};

    /// Run the handler on `request` and return the status and body.
    fn answer(request: &HttpRequest<'_>) -> (StatusCode, std::vec::Vec<u8>) {
        let mut handler = SimpleHandler;
        let mut idle = Idle;
        let mut buf = [0u8; 0];
        let mut body = RequestBody::new(&mut idle, BodyParts::empty(&mut buf));
        let response =
            futures_lite::future::block_on(handler.handle_request(request, &mut body)).unwrap();
        (response.status_code, response.body.to_vec())
    }

    /// A connection with nothing to read, for requests without a body.
    struct Idle;

    impl ErrorType for Idle {
        type Error = Infallible;
    }

    impl Read for Idle {
        async fn read(&mut self, _buf: &mut [u8]) -> Result<usize, Self::Error> {
            Ok(0)
        }
    }

    impl Write for Idle {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            Ok(buf.len())
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    fn get(path: &str) -> HttpRequest<'_> {
        HttpRequest {
            method: HttpMethod::GET,
            path,
            version: "HTTP/1.1",
            headers: Vec::new(),
            body: b"",
        }
    }

    #[test]
    fn test_simple_handler() {
        let (status, body) = answer(&get("/"));
        assert_eq!(status, StatusCode::Ok);
        assert_eq!(body, b"<h1>Hello from nanofish HTTP server!</h1>");

        let (status, body) = answer(&get("/health"));
        assert_eq!(status, StatusCode::Ok);
        assert_eq!(body, b"{\"status\":\"ok\"}");

        let (status, body) = answer(&get("/nonexistent"));
        assert_eq!(status, StatusCode::NotFound);
        assert_eq!(body, b"404 Not Found");
    }
}
