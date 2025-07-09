/// HTTP/1.1 status codes as defined in RFC 2616 section 10
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
    /// Any other (unknown or non-standard) status code
    Other(u16),
}

#[allow(dead_code)]
impl StatusCode {
    /// Returns the numeric status code as u16.
    #[must_use]
    pub fn as_u16(self) -> u16 {
        match self {
            StatusCode::Continue => 100,
            StatusCode::SwitchingProtocols => 101,
            StatusCode::Ok => 200,
            StatusCode::Created => 201,
            StatusCode::Accepted => 202,
            StatusCode::NonAuthoritativeInformation => 203,
            StatusCode::NoContent => 204,
            StatusCode::ResetContent => 205,
            StatusCode::PartialContent => 206,
            StatusCode::MultipleChoices => 300,
            StatusCode::MovedPermanently => 301,
            StatusCode::Found => 302,
            StatusCode::SeeOther => 303,
            StatusCode::NotModified => 304,
            StatusCode::UseProxy => 305,
            StatusCode::TemporaryRedirect => 307,
            StatusCode::BadRequest => 400,
            StatusCode::Unauthorized => 401,
            StatusCode::PaymentRequired => 402,
            StatusCode::Forbidden => 403,
            StatusCode::NotFound => 404,
            StatusCode::MethodNotAllowed => 405,
            StatusCode::NotAcceptable => 406,
            StatusCode::ProxyAuthenticationRequired => 407,
            StatusCode::RequestTimeout => 408,
            StatusCode::Conflict => 409,
            StatusCode::Gone => 410,
            StatusCode::LengthRequired => 411,
            StatusCode::PreconditionFailed => 412,
            StatusCode::RequestEntityTooLarge => 413,
            StatusCode::RequestUriTooLong => 414,
            StatusCode::UnsupportedMediaType => 415,
            StatusCode::RequestedRangeNotSatisfiable => 416,
            StatusCode::ExpectationFailed => 417,
            StatusCode::InternalServerError => 500,
            StatusCode::NotImplemented => 501,
            StatusCode::BadGateway => 502,
            StatusCode::ServiceUnavailable => 503,
            StatusCode::GatewayTimeout => 504,
            StatusCode::HttpVersionNotSupported => 505,
            StatusCode::Other(code) => code,
        }
    }
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
            StatusCode::Other(_) => "Other",
        }
    }

    /// Check if the status code indicates success (2xx status codes)
    #[must_use]
    pub fn is_success(self) -> bool {
        let code = self.as_u16();
        (200..300).contains(&code)
    }

    /// Check if the status code is a client error (4xx status codes)
    #[must_use]
    pub fn is_client_error(self) -> bool {
        let code = self.as_u16();
        (400..500).contains(&code)
    }

    /// Check if the status code is a server error (5xx status codes)
    #[must_use]
    pub fn is_server_error(self) -> bool {
        let code = self.as_u16();
        (500..600).contains(&code)
    }
}

impl From<u16> for StatusCode {
    fn from(code: u16) -> Self {
        match code {
            100 => StatusCode::Continue,
            101 => StatusCode::SwitchingProtocols,
            200 => StatusCode::Ok,
            201 => StatusCode::Created,
            202 => StatusCode::Accepted,
            203 => StatusCode::NonAuthoritativeInformation,
            204 => StatusCode::NoContent,
            205 => StatusCode::ResetContent,
            206 => StatusCode::PartialContent,
            300 => StatusCode::MultipleChoices,
            301 => StatusCode::MovedPermanently,
            302 => StatusCode::Found,
            303 => StatusCode::SeeOther,
            304 => StatusCode::NotModified,
            305 => StatusCode::UseProxy,
            307 => StatusCode::TemporaryRedirect,
            400 => StatusCode::BadRequest,
            401 => StatusCode::Unauthorized,
            402 => StatusCode::PaymentRequired,
            403 => StatusCode::Forbidden,
            404 => StatusCode::NotFound,
            405 => StatusCode::MethodNotAllowed,
            406 => StatusCode::NotAcceptable,
            407 => StatusCode::ProxyAuthenticationRequired,
            408 => StatusCode::RequestTimeout,
            409 => StatusCode::Conflict,
            410 => StatusCode::Gone,
            411 => StatusCode::LengthRequired,
            412 => StatusCode::PreconditionFailed,
            413 => StatusCode::RequestEntityTooLarge,
            414 => StatusCode::RequestUriTooLong,
            415 => StatusCode::UnsupportedMediaType,
            416 => StatusCode::RequestedRangeNotSatisfiable,
            417 => StatusCode::ExpectationFailed,
            500 => StatusCode::InternalServerError,
            501 => StatusCode::NotImplemented,
            502 => StatusCode::BadGateway,
            503 => StatusCode::ServiceUnavailable,
            504 => StatusCode::GatewayTimeout,
            505 => StatusCode::HttpVersionNotSupported,
            other => StatusCode::Other(other),
        }
    }
}

impl TryFrom<&str> for StatusCode {
    type Error = crate::Error;

    /// Parse a status code from a string slice.
    ///
    /// # Examples
    ///
    /// ```
    /// use nanofish::StatusCode;
    ///
    /// let code: StatusCode = "200".try_into().unwrap();
    /// assert_eq!(code, StatusCode::Ok);
    ///
    /// let result: Result<StatusCode, _> = "999".try_into();
    /// assert_eq!(result.unwrap(), StatusCode::Other(999));
    /// ```
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if let Ok(code) = value.parse::<u16>() {
            Ok(StatusCode::from(code))
        } else {
            Err(crate::Error::InvalidStatusCode)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_u16_known_codes() {
        // Test a few known codes using From
        let code: StatusCode = 200_u16.into();
        assert_eq!(code, StatusCode::Ok);

        let code: StatusCode = 404_u16.into();
        assert_eq!(code, StatusCode::NotFound);

        let code: StatusCode = 500_u16.into();
        assert_eq!(code, StatusCode::InternalServerError);

        let code: StatusCode = 100_u16.into();
        assert_eq!(code, StatusCode::Continue);

        let code: StatusCode = 307_u16.into();
        assert_eq!(code, StatusCode::TemporaryRedirect);
    }

    #[test]
    fn test_from_u16_unknown_code() {
        // Test unknown codes using From
        let code: StatusCode = 999_u16.into();
        assert_eq!(code, StatusCode::Other(999));
        let code: StatusCode = 150_u16.into();
        assert_eq!(code, StatusCode::Other(150));
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
        assert_eq!(StatusCode::Ok.as_u16(), 200);
        assert_eq!(StatusCode::NotFound.as_u16(), 404);
        assert_eq!(StatusCode::InternalServerError.as_u16(), 500);
        assert_eq!(StatusCode::Continue.as_u16(), 100);
        assert_eq!(StatusCode::TemporaryRedirect.as_u16(), 307);
    }

    #[test]
    fn test_is_success() {
        assert!(StatusCode::Ok.is_success());
        assert!(StatusCode::Created.is_success());
        assert!(StatusCode::Accepted.is_success());
        assert!(StatusCode::NoContent.is_success());

        assert!(!StatusCode::Continue.is_success());
        assert!(!StatusCode::NotFound.is_success());
        assert!(!StatusCode::InternalServerError.is_success());
        assert!(!StatusCode::MovedPermanently.is_success());
    }

    #[test]
    fn test_is_client_error() {
        assert!(StatusCode::BadRequest.is_client_error());
        assert!(StatusCode::Unauthorized.is_client_error());
        assert!(StatusCode::Forbidden.is_client_error());
        assert!(StatusCode::NotFound.is_client_error());
        assert!(StatusCode::MethodNotAllowed.is_client_error());

        assert!(!StatusCode::Ok.is_client_error());
        assert!(!StatusCode::Continue.is_client_error());
        assert!(!StatusCode::InternalServerError.is_client_error());
        assert!(!StatusCode::MovedPermanently.is_client_error());
    }

    #[test]
    fn test_is_server_error() {
        assert!(StatusCode::InternalServerError.is_server_error());
        assert!(StatusCode::NotImplemented.is_server_error());
        assert!(StatusCode::BadGateway.is_server_error());
        assert!(StatusCode::ServiceUnavailable.is_server_error());
        assert!(StatusCode::GatewayTimeout.is_server_error());

        assert!(!StatusCode::Ok.is_server_error());
        assert!(!StatusCode::Continue.is_server_error());
        assert!(!StatusCode::NotFound.is_server_error());
        assert!(!StatusCode::MovedPermanently.is_server_error());
    }

    #[test]
    fn test_try_from_str_valid() {
        // Test valid status code strings
        let code: StatusCode = "200".try_into().unwrap();
        assert_eq!(code, StatusCode::Ok);

        let code: StatusCode = "404".try_into().unwrap();
        assert_eq!(code, StatusCode::NotFound);

        let code: StatusCode = "500".try_into().unwrap();
        assert_eq!(code, StatusCode::InternalServerError);

        let code: StatusCode = "100".try_into().unwrap();
        assert_eq!(code, StatusCode::Continue);
    }

    #[test]
    fn test_try_from_str_invalid() {
        // Only non-numeric/invalid strings should error
        let result: Result<StatusCode, _> = "abc".try_into();
        assert!(result.is_err());

        let result: Result<StatusCode, _> = "".try_into();
        assert!(result.is_err());

        let result: Result<StatusCode, _> = "12345".try_into();
        assert_eq!(result.unwrap(), StatusCode::Other(12345));

        // Numeric strings always succeed, even if unknown
        let result: Result<StatusCode, _> = "999".try_into();
        assert_eq!(result.unwrap(), StatusCode::Other(999));
        let result: Result<StatusCode, _> = "150".try_into();
        assert_eq!(result.unwrap(), StatusCode::Other(150));
    }

    #[test]
    fn test_try_from_u16_valid() {
        // Test valid status codes
        let code: StatusCode = 200_u16.into();
        assert_eq!(code, StatusCode::Ok);

        let code: StatusCode = 404_u16.into();
        assert_eq!(code, StatusCode::NotFound);

        let code: StatusCode = 500_u16.into();
        assert_eq!(code, StatusCode::InternalServerError);

        let code: StatusCode = 100_u16.into();
        assert_eq!(code, StatusCode::Continue);

        let code: StatusCode = 307_u16.into();
        assert_eq!(code, StatusCode::TemporaryRedirect);
    }
}
