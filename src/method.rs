/// HTTP Methods supported by the client
///
/// This enum represents the standard HTTP methods that can be used
/// when making requests with the `HttpClient`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// Error type for invalid HTTP methods
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidHttpMethod;

impl core::fmt::Display for InvalidHttpMethod {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Invalid HTTP method")
    }
}

impl HttpMethod {
    #[must_use]
    /// Returns the string representation of the HTTP method.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GET => "GET",
            Self::POST => "POST",
            Self::PUT => "PUT",
            Self::DELETE => "DELETE",
            Self::PATCH => "PATCH",
            Self::CONNECT => "CONNECT",
            Self::OPTIONS => "OPTIONS",
            Self::TRACE => "TRACE",
            Self::HEAD => "HEAD",
        }
    }
}

impl TryFrom<&str> for HttpMethod {
    type Error = InvalidHttpMethod;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "GET" => Ok(Self::GET),
            "POST" => Ok(Self::POST),
            "PUT" => Ok(Self::PUT),
            "DELETE" => Ok(Self::DELETE),
            "PATCH" => Ok(Self::PATCH),
            "HEAD" => Ok(Self::HEAD),
            "OPTIONS" => Ok(Self::OPTIONS),
            "TRACE" => Ok(Self::TRACE),
            "CONNECT" => Ok(Self::CONNECT),
            _ => Err(InvalidHttpMethod),
        }
    }
}

impl TryFrom<&[u8]> for HttpMethod {
    type Error = InvalidHttpMethod;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        match value {
            b"GET" => Ok(Self::GET),
            b"POST" => Ok(Self::POST),
            b"PUT" => Ok(Self::PUT),
            b"DELETE" => Ok(Self::DELETE),
            b"PATCH" => Ok(Self::PATCH),
            b"HEAD" => Ok(Self::HEAD),
            b"OPTIONS" => Ok(Self::OPTIONS),
            b"TRACE" => Ok(Self::TRACE),
            b"CONNECT" => Ok(Self::CONNECT),
            _ => Err(InvalidHttpMethod),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_method_as_str() {
        assert_eq!(HttpMethod::GET.as_str(), "GET");
        assert_eq!(HttpMethod::POST.as_str(), "POST");
        assert_eq!(HttpMethod::PUT.as_str(), "PUT");
        assert_eq!(HttpMethod::DELETE.as_str(), "DELETE");
        assert_eq!(HttpMethod::PATCH.as_str(), "PATCH");
        assert_eq!(HttpMethod::CONNECT.as_str(), "CONNECT");
        assert_eq!(HttpMethod::OPTIONS.as_str(), "OPTIONS");
        assert_eq!(HttpMethod::TRACE.as_str(), "TRACE");
        assert_eq!(HttpMethod::HEAD.as_str(), "HEAD");
    }

    #[test]
    fn test_try_from_str() {
        // Test valid HTTP methods
        assert_eq!(HttpMethod::try_from("GET"), Ok(HttpMethod::GET));
        assert_eq!(HttpMethod::try_from("POST"), Ok(HttpMethod::POST));
        assert_eq!(HttpMethod::try_from("PUT"), Ok(HttpMethod::PUT));
        assert_eq!(HttpMethod::try_from("DELETE"), Ok(HttpMethod::DELETE));
        assert_eq!(HttpMethod::try_from("PATCH"), Ok(HttpMethod::PATCH));
        assert_eq!(HttpMethod::try_from("HEAD"), Ok(HttpMethod::HEAD));
        assert_eq!(HttpMethod::try_from("OPTIONS"), Ok(HttpMethod::OPTIONS));
        assert_eq!(HttpMethod::try_from("TRACE"), Ok(HttpMethod::TRACE));
        assert_eq!(HttpMethod::try_from("CONNECT"), Ok(HttpMethod::CONNECT));

        // Test invalid HTTP methods
        assert_eq!(HttpMethod::try_from("get"), Err(InvalidHttpMethod));
        assert_eq!(HttpMethod::try_from("INVALID"), Err(InvalidHttpMethod));
        assert_eq!(HttpMethod::try_from(""), Err(InvalidHttpMethod));
        assert_eq!(HttpMethod::try_from("123"), Err(InvalidHttpMethod));
    }

    #[test]
    fn test_try_from_bytes() {
        // Test valid HTTP methods
        assert_eq!(HttpMethod::try_from(b"GET".as_slice()), Ok(HttpMethod::GET));
        assert_eq!(
            HttpMethod::try_from(b"POST".as_slice()),
            Ok(HttpMethod::POST)
        );
        assert_eq!(HttpMethod::try_from(b"PUT".as_slice()), Ok(HttpMethod::PUT));
        assert_eq!(
            HttpMethod::try_from(b"DELETE".as_slice()),
            Ok(HttpMethod::DELETE)
        );
        assert_eq!(
            HttpMethod::try_from(b"PATCH".as_slice()),
            Ok(HttpMethod::PATCH)
        );
        assert_eq!(
            HttpMethod::try_from(b"HEAD".as_slice()),
            Ok(HttpMethod::HEAD)
        );
        assert_eq!(
            HttpMethod::try_from(b"OPTIONS".as_slice()),
            Ok(HttpMethod::OPTIONS)
        );
        assert_eq!(
            HttpMethod::try_from(b"TRACE".as_slice()),
            Ok(HttpMethod::TRACE)
        );
        assert_eq!(
            HttpMethod::try_from(b"CONNECT".as_slice()),
            Ok(HttpMethod::CONNECT)
        );

        // Test invalid HTTP methods
        assert_eq!(
            HttpMethod::try_from(b"get".as_slice()),
            Err(InvalidHttpMethod)
        );
        assert_eq!(
            HttpMethod::try_from(b"INVALID".as_slice()),
            Err(InvalidHttpMethod)
        );
        assert_eq!(HttpMethod::try_from(b"".as_slice()), Err(InvalidHttpMethod));
        assert_eq!(
            HttpMethod::try_from(b"123".as_slice()),
            Err(InvalidHttpMethod)
        );
    }

    #[test]
    fn test_invalid_http_method_display() {
        let error = InvalidHttpMethod;
        assert_eq!(format!("{error}"), "Invalid HTTP method");
    }

    #[test]
    fn test_roundtrip_str_conversion() {
        let methods = [
            HttpMethod::GET,
            HttpMethod::POST,
            HttpMethod::PUT,
            HttpMethod::DELETE,
            HttpMethod::PATCH,
            HttpMethod::HEAD,
            HttpMethod::OPTIONS,
            HttpMethod::TRACE,
            HttpMethod::CONNECT,
        ];

        for method in &methods {
            let str_repr = method.as_str();
            let parsed = HttpMethod::try_from(str_repr).unwrap();
            assert_eq!(*method, parsed);
        }
    }
}
