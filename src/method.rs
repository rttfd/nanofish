/// HTTP Methods supported by the client
///
/// This enum represents the standard HTTP methods that can be used
/// when making requests with the `HttpClient`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HttpMethod {
    /// The GET method requests a representation of the specified resource.
    /// Requests using GET should only retrieve data.
    GET,
    /// The POST method is used to submit an entity to the specified resource,
    /// often causing a change in state or side effects on the server.
    POST,
    /// The PUT method replaces all current representations of the target
    /// resource with the request payload.
    PUT,
    /// The DELETE method deletes the specified resource.
    DELETE,
    /// The PATCH method is used to apply partial modifications to a resource.
    PATCH,
    /// The CONNECT method establishes a tunnel to the server identified by the target resource.
    CONNECT,
    /// The OPTIONS method is used to describe the communication options for the target resource.
    OPTIONS,
    /// The TRACE method performs a message loop-back test along the path to the target resource.
    TRACE,
    /// The HEAD method asks for a response identical to that of a GET request,
    /// but without the response body.
    HEAD,
}

impl HttpMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            HttpMethod::GET => "GET",
            HttpMethod::POST => "POST",
            HttpMethod::PUT => "PUT",
            HttpMethod::DELETE => "DELETE",
            HttpMethod::PATCH => "PATCH",
            HttpMethod::CONNECT => "CONNECT",
            HttpMethod::OPTIONS => "OPTIONS",
            HttpMethod::TRACE => "TRACE",
            HttpMethod::HEAD => "HEAD",
        }
    }
}
