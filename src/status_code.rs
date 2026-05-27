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

impl StatusCode {
    /// Returns the numeric status code as u16.
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        match self {
            Self::Continue => 100,
            Self::SwitchingProtocols => 101,
            Self::Ok => 200,
            Self::Created => 201,
            Self::Accepted => 202,
            Self::NonAuthoritativeInformation => 203,
            Self::NoContent => 204,
            Self::ResetContent => 205,
            Self::PartialContent => 206,
            Self::MultipleChoices => 300,
            Self::MovedPermanently => 301,
            Self::Found => 302,
            Self::SeeOther => 303,
            Self::NotModified => 304,
            Self::UseProxy => 305,
            Self::TemporaryRedirect => 307,
            Self::BadRequest => 400,
            Self::Unauthorized => 401,
            Self::PaymentRequired => 402,
            Self::Forbidden => 403,
            Self::NotFound => 404,
            Self::MethodNotAllowed => 405,
            Self::NotAcceptable => 406,
            Self::ProxyAuthenticationRequired => 407,
            Self::RequestTimeout => 408,
            Self::Conflict => 409,
            Self::Gone => 410,
            Self::LengthRequired => 411,
            Self::PreconditionFailed => 412,
            Self::RequestEntityTooLarge => 413,
            Self::RequestUriTooLong => 414,
            Self::UnsupportedMediaType => 415,
            Self::RequestedRangeNotSatisfiable => 416,
            Self::ExpectationFailed => 417,
            Self::InternalServerError => 500,
            Self::NotImplemented => 501,
            Self::BadGateway => 502,
            Self::ServiceUnavailable => 503,
            Self::GatewayTimeout => 504,
            Self::HttpVersionNotSupported => 505,
            Self::Other(code) => code,
        }
    }
    /// Returns the status code text.
    #[must_use]
    pub const fn text(self) -> &'static str {
        match self {
            // 1xx
            Self::Continue => "Continue",
            Self::SwitchingProtocols => "Switching Protocols",
            // 2xx
            Self::Ok => "OK",
            Self::Created => "Created",
            Self::Accepted => "Accepted",
            Self::NonAuthoritativeInformation => "Non-Authoritative Information",
            Self::NoContent => "No Content",
            Self::ResetContent => "Reset Content",
            Self::PartialContent => "Partial Content",
            // 3xx
            Self::MultipleChoices => "Multiple Choices",
            Self::MovedPermanently => "Moved Permanently",
            Self::Found => "Found",
            Self::SeeOther => "See Other",
            Self::NotModified => "Not Modified",
            Self::UseProxy => "Use Proxy",
            Self::TemporaryRedirect => "Temporary Redirect",
            // 4xx
            Self::BadRequest => "Bad Request",
            Self::Unauthorized => "Unauthorized",
            Self::PaymentRequired => "Payment Required",
            Self::Forbidden => "Forbidden",
            Self::NotFound => "Not Found",
            Self::MethodNotAllowed => "Method Not Allowed",
            Self::NotAcceptable => "Not Acceptable",
            Self::ProxyAuthenticationRequired => "Proxy Authentication Required",
            Self::RequestTimeout => "Request Timeout",
            Self::Conflict => "Conflict",
            Self::Gone => "Gone",
            Self::LengthRequired => "Length Required",
            Self::PreconditionFailed => "Precondition Failed",
            Self::RequestEntityTooLarge => "Request Entity Too Large",
            Self::RequestUriTooLong => "Request-URI Too Long",
            Self::UnsupportedMediaType => "Unsupported Media Type",
            Self::RequestedRangeNotSatisfiable => "Requested Range Not Satisfiable",
            Self::ExpectationFailed => "Expectation Failed",
            // 5xx
            Self::InternalServerError => "Internal Server Error",
            Self::NotImplemented => "Not Implemented",
            Self::BadGateway => "Bad Gateway",
            Self::ServiceUnavailable => "Service Unavailable",
            Self::GatewayTimeout => "Gateway Timeout",
            Self::HttpVersionNotSupported => "HTTP Version Not Supported",
            Self::Other(_) => "Other",
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
            100 => Self::Continue,
            101 => Self::SwitchingProtocols,
            200 => Self::Ok,
            201 => Self::Created,
            202 => Self::Accepted,
            203 => Self::NonAuthoritativeInformation,
            204 => Self::NoContent,
            205 => Self::ResetContent,
            206 => Self::PartialContent,
            300 => Self::MultipleChoices,
            301 => Self::MovedPermanently,
            302 => Self::Found,
            303 => Self::SeeOther,
            304 => Self::NotModified,
            305 => Self::UseProxy,
            307 => Self::TemporaryRedirect,
            400 => Self::BadRequest,
            401 => Self::Unauthorized,
            402 => Self::PaymentRequired,
            403 => Self::Forbidden,
            404 => Self::NotFound,
            405 => Self::MethodNotAllowed,
            406 => Self::NotAcceptable,
            407 => Self::ProxyAuthenticationRequired,
            408 => Self::RequestTimeout,
            409 => Self::Conflict,
            410 => Self::Gone,
            411 => Self::LengthRequired,
            412 => Self::PreconditionFailed,
            413 => Self::RequestEntityTooLarge,
            414 => Self::RequestUriTooLong,
            415 => Self::UnsupportedMediaType,
            416 => Self::RequestedRangeNotSatisfiable,
            417 => Self::ExpectationFailed,
            500 => Self::InternalServerError,
            501 => Self::NotImplemented,
            502 => Self::BadGateway,
            503 => Self::ServiceUnavailable,
            504 => Self::GatewayTimeout,
            505 => Self::HttpVersionNotSupported,
            other => Self::Other(other),
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
        value
            .parse::<u16>()
            .map_or(Err(crate::Error::InvalidStatusCode), |code| {
                Ok(Self::from(code))
            })
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
