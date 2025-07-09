/// HTTP/1.1 status codes as defined in RFC 2616 section 10
/// Predefined HTTP status codes as per RFC 2616 section 10.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum StatusCode {
    // 1xx Informational
    /// 100 Continue: The server has received the request headers, and the client should proceed to send the request body.
    Continue = 100,
    /// 101 Switching Protocols: The requester has asked the server to switch protocols.
    SwitchingProtocols = 101,

    // 2xx Success
    /// 200 OK: The request has succeeded.
    Ok = 200,
    /// 201 Created: The request has been fulfilled and resulted in a new resource being created.
    Created = 201,
    /// 202 Accepted: The request has been accepted for processing, but the processing has not been completed.
    Accepted = 202,
    /// 203 Non-Authoritative Information: The server successfully processed the request, but is returning information that may be from another source.
    NonAuthoritativeInformation = 203,
    /// 204 No Content: The server successfully processed the request, but is not returning any content.
    NoContent = 204,
    /// 205 Reset Content: The server successfully processed the request, but is not returning any content and requires that the requester reset the document view.
    ResetContent = 205,
    /// 206 Partial Content: The server is delivering only part of the resource due to a range header sent by the client.
    PartialContent = 206,

    // 3xx Redirection
    /// 300 Multiple Choices: Indicates multiple options for the resource from which the client may choose.
    MultipleChoices = 300,
    /// 301 Moved Permanently: This and all future requests should be directed to the given URI.
    MovedPermanently = 301,
    /// 302 Found: The resource was found but at a different URI.
    Found = 302,
    /// 303 See Other: The response to the request can be found under another URI using a GET method.
    SeeOther = 303,
    /// 304 Not Modified: Indicates that the resource has not been modified since the version specified by the request headers.
    NotModified = 304,
    /// 305 Use Proxy: The requested resource is available only through a proxy.
    UseProxy = 305,
    // 306 is unused
    /// 307 Temporary Redirect: The request should be repeated with another URI, but future requests should still use the original URI.
    TemporaryRedirect = 307,

    // 4xx Client Error
    /// 400 Bad Request: The server could not understand the request due to invalid syntax.
    BadRequest = 400,
    /// 401 Unauthorized: The client must authenticate itself to get the requested response.
    Unauthorized = 401,
    /// 402 Payment Required: Reserved for future use.
    PaymentRequired = 402,
    /// 403 Forbidden: The client does not have access rights to the content.
    Forbidden = 403,
    /// 404 Not Found: The server can not find the requested resource.
    NotFound = 404,
    /// 405 Method Not Allowed: The request method is known by the server but is not supported by the target resource.
    MethodNotAllowed = 405,
    /// 406 Not Acceptable: The server cannot produce a response matching the list of acceptable values defined in the request's headers.
    NotAcceptable = 406,
    /// 407 Proxy Authentication Required: The client must first authenticate itself with the proxy.
    ProxyAuthenticationRequired = 407,
    /// 408 Request Timeout: The server timed out waiting for the request.
    RequestTimeout = 408,
    /// 409 Conflict: The request could not be completed due to a conflict with the current state of the resource.
    Conflict = 409,
    /// 410 Gone: The resource requested is no longer available and will not be available again.
    Gone = 410,
    /// 411 Length Required: The request did not specify the length of its content, which is required by the requested resource.
    LengthRequired = 411,
    /// 412 Precondition Failed: The server does not meet one of the preconditions that the requester put on the request.
    PreconditionFailed = 412,
    /// 413 Request Entity Too Large: The request is larger than the server is willing or able to process.
    RequestEntityTooLarge = 413,
    /// 414 Request-URI Too Long: The URI provided was too long for the server to process.
    RequestUriTooLong = 414,
    /// 415 Unsupported Media Type: The request entity has a media type which the server or resource does not support.
    UnsupportedMediaType = 415,
    /// 416 Requested Range Not Satisfiable: The client has asked for a portion of the file, but the server cannot supply that portion.
    RequestedRangeNotSatisfiable = 416,
    /// 417 Expectation Failed: The server cannot meet the requirements of the Expect request-header field.
    ExpectationFailed = 417,

    // 5xx Server Error
    /// 500 Internal Server Error: The server has encountered a situation it doesn't know how to handle.
    InternalServerError = 500,
    /// 501 Not Implemented: The request method is not supported by the server and cannot be handled.
    NotImplemented = 501,
    /// 502 Bad Gateway: The server, while acting as a gateway or proxy, received an invalid response from the upstream server.
    BadGateway = 502,
    /// 503 Service Unavailable: The server is not ready to handle the request.
    ServiceUnavailable = 503,
    /// 504 Gateway Timeout: The server is acting as a gateway and cannot get a response in time.
    GatewayTimeout = 504,
    /// 505 HTTP Version Not Supported: The HTTP version used in the request is not supported by the server.
    HttpVersionNotSupported = 505,
}

#[allow(dead_code)]
impl StatusCode {
    /// Returns the status code text.
    #[must_use]
    pub fn text(self) -> &'static str {
        match self {
            // 1xx
            StatusCode::Continue => "Continue",
            StatusCode::SwitchingProtocols => "Switching Protocols",
            // 2xx
            StatusCode::Ok => "OK",
            StatusCode::Created => "Created",
            StatusCode::Accepted => "Accepted",
            StatusCode::NonAuthoritativeInformation => "Non-Authoritative Information",
            StatusCode::NoContent => "No Content",
            StatusCode::ResetContent => "Reset Content",
            StatusCode::PartialContent => "Partial Content",
            // 3xx
            StatusCode::MultipleChoices => "Multiple Choices",
            StatusCode::MovedPermanently => "Moved Permanently",
            StatusCode::Found => "Found",
            StatusCode::SeeOther => "See Other",
            StatusCode::NotModified => "Not Modified",
            StatusCode::UseProxy => "Use Proxy",
            StatusCode::TemporaryRedirect => "Temporary Redirect",
            // 4xx
            StatusCode::BadRequest => "Bad Request",
            StatusCode::Unauthorized => "Unauthorized",
            StatusCode::PaymentRequired => "Payment Required",
            StatusCode::Forbidden => "Forbidden",
            StatusCode::NotFound => "Not Found",
            StatusCode::MethodNotAllowed => "Method Not Allowed",
            StatusCode::NotAcceptable => "Not Acceptable",
            StatusCode::ProxyAuthenticationRequired => "Proxy Authentication Required",
            StatusCode::RequestTimeout => "Request Timeout",
            StatusCode::Conflict => "Conflict",
            StatusCode::Gone => "Gone",
            StatusCode::LengthRequired => "Length Required",
            StatusCode::PreconditionFailed => "Precondition Failed",
            StatusCode::RequestEntityTooLarge => "Request Entity Too Large",
            StatusCode::RequestUriTooLong => "Request-URI Too Long",
            StatusCode::UnsupportedMediaType => "Unsupported Media Type",
            StatusCode::RequestedRangeNotSatisfiable => "Requested Range Not Satisfiable",
            StatusCode::ExpectationFailed => "Expectation Failed",
            // 5xx
            StatusCode::InternalServerError => "Internal Server Error",
            StatusCode::NotImplemented => "Not Implemented",
            StatusCode::BadGateway => "Bad Gateway",
            StatusCode::ServiceUnavailable => "Service Unavailable",
            StatusCode::GatewayTimeout => "Gateway Timeout",
            StatusCode::HttpVersionNotSupported => "HTTP Version Not Supported",
        }
    }

    /// Try to convert a u16 to a `StatusCode`.
    #[must_use]
    pub fn from_u16(code: u16) -> Option<StatusCode> {
        match code {
            100 => Some(StatusCode::Continue),
            101 => Some(StatusCode::SwitchingProtocols),
            200 => Some(StatusCode::Ok),
            201 => Some(StatusCode::Created),
            202 => Some(StatusCode::Accepted),
            203 => Some(StatusCode::NonAuthoritativeInformation),
            204 => Some(StatusCode::NoContent),
            205 => Some(StatusCode::ResetContent),
            206 => Some(StatusCode::PartialContent),
            300 => Some(StatusCode::MultipleChoices),
            301 => Some(StatusCode::MovedPermanently),
            302 => Some(StatusCode::Found),
            303 => Some(StatusCode::SeeOther),
            304 => Some(StatusCode::NotModified),
            305 => Some(StatusCode::UseProxy),
            307 => Some(StatusCode::TemporaryRedirect),
            400 => Some(StatusCode::BadRequest),
            401 => Some(StatusCode::Unauthorized),
            402 => Some(StatusCode::PaymentRequired),
            403 => Some(StatusCode::Forbidden),
            404 => Some(StatusCode::NotFound),
            405 => Some(StatusCode::MethodNotAllowed),
            406 => Some(StatusCode::NotAcceptable),
            407 => Some(StatusCode::ProxyAuthenticationRequired),
            408 => Some(StatusCode::RequestTimeout),
            409 => Some(StatusCode::Conflict),
            410 => Some(StatusCode::Gone),
            411 => Some(StatusCode::LengthRequired),
            412 => Some(StatusCode::PreconditionFailed),
            413 => Some(StatusCode::RequestEntityTooLarge),
            414 => Some(StatusCode::RequestUriTooLong),
            415 => Some(StatusCode::UnsupportedMediaType),
            416 => Some(StatusCode::RequestedRangeNotSatisfiable),
            417 => Some(StatusCode::ExpectationFailed),
            500 => Some(StatusCode::InternalServerError),
            501 => Some(StatusCode::NotImplemented),
            502 => Some(StatusCode::BadGateway),
            503 => Some(StatusCode::ServiceUnavailable),
            504 => Some(StatusCode::GatewayTimeout),
            505 => Some(StatusCode::HttpVersionNotSupported),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_u16_known_codes() {
        // Test a few known codes
        assert_eq!(StatusCode::from_u16(200), Some(StatusCode::Ok));
        assert_eq!(StatusCode::from_u16(404), Some(StatusCode::NotFound));
        assert_eq!(
            StatusCode::from_u16(500),
            Some(StatusCode::InternalServerError)
        );
        assert_eq!(StatusCode::from_u16(100), Some(StatusCode::Continue));
        assert_eq!(
            StatusCode::from_u16(307),
            Some(StatusCode::TemporaryRedirect)
        );
    }

    #[test]
    fn test_from_u16_unknown_code() {
        // Test an unknown code
        assert_eq!(StatusCode::from_u16(999), None);
        assert_eq!(StatusCode::from_u16(150), None);
    }

    #[test]
    fn test_text() {
        assert_eq!(StatusCode::Ok.text(), "OK");
        assert_eq!(StatusCode::NotFound.text(), "Not Found");
        assert_eq!(
            StatusCode::InternalServerError.text(),
            "Internal Server Error"
        );
        assert_eq!(StatusCode::BadRequest.text(), "Bad Request");
        assert_eq!(StatusCode::TemporaryRedirect.text(), "Temporary Redirect");
    }

    #[test]
    fn test_enum_values_match_code() {
        // Ensure the discriminant matches the HTTP code
        assert_eq!(StatusCode::Ok as u16, 200);
        assert_eq!(StatusCode::NotFound as u16, 404);
        assert_eq!(StatusCode::InternalServerError as u16, 500);
        assert_eq!(StatusCode::Continue as u16, 100);
        assert_eq!(StatusCode::TemporaryRedirect as u16, 307);
    }
}
