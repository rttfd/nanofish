use crate::{
    error::Error,
    header::{HttpHeader, mime_types},
    method::HttpMethod,
    protocol::MAX_HEADERS,
};
use heapless::Vec;

/// Collects the method, headers and body of a request with a fluent API.
///
/// The builder does not know where the request goes: pass it to
/// [`HttpClient::send`](crate::HttpClient::send) together with a host and
/// path, or to `EmbassyHttpClient::send` with a URL.
///
/// Headers and body are borrowed, so values that are computed at runtime (such
/// as the result of [`basic_auth`](crate::encode::basic_auth)) have to outlive
/// the builder.
///
/// ```
/// use nanofish::{HttpMethod, RequestBuilder, encode::bearer_auth};
///
/// let token = bearer_auth::<64>("s3cret").unwrap();
/// let request = RequestBuilder::post()
///     .authorization(&token)
///     .json(r#"{"on":true}"#);
///
/// assert_eq!(request.method(), HttpMethod::POST);
/// assert_eq!(request.headers().len(), 2);
/// assert_eq!(request.body_bytes(), Some(br#"{"on":true}"#.as_slice()));
/// assert!(request.check().is_ok());
/// ```
#[derive(Debug, Clone)]
pub struct RequestBuilder<'a> {
    method: HttpMethod,
    headers: Vec<HttpHeader<'a>, MAX_HEADERS>,
    body: Option<&'a [u8]>,
    /// More headers were added than fit; reported by [`Self::check`].
    overflow: bool,
}

impl<'a> RequestBuilder<'a> {
    /// Start a request with `method`, no headers and no body.
    #[must_use]
    pub const fn new(method: HttpMethod) -> Self {
        Self {
            method,
            headers: Vec::new(),
            body: None,
            overflow: false,
        }
    }

    /// Start a `GET` request.
    #[must_use]
    pub const fn get() -> Self {
        Self::new(HttpMethod::GET)
    }

    /// Start a `POST` request.
    #[must_use]
    pub const fn post() -> Self {
        Self::new(HttpMethod::POST)
    }

    /// Start a `PUT` request.
    #[must_use]
    pub const fn put() -> Self {
        Self::new(HttpMethod::PUT)
    }

    /// Start a `PATCH` request.
    #[must_use]
    pub const fn patch() -> Self {
        Self::new(HttpMethod::PATCH)
    }

    /// Start a `DELETE` request.
    #[must_use]
    pub const fn delete() -> Self {
        Self::new(HttpMethod::DELETE)
    }

    /// Start a `HEAD` request.
    #[must_use]
    pub const fn head() -> Self {
        Self::new(HttpMethod::HEAD)
    }

    /// Add a header.
    ///
    /// More than [`MAX_HEADERS`] headers is reported by [`Self::check`] (and by
    /// the `send` methods) as [`Error::BufferOverflow`].
    #[must_use]
    pub fn header(mut self, name: &'a str, value: &'a str) -> Self {
        self.push(HttpHeader::new(name, value));
        self
    }

    /// Add several headers.
    #[must_use]
    pub fn headers_from(mut self, headers: &[HttpHeader<'a>]) -> Self {
        for header in headers {
            self.push(header.clone());
        }
        self
    }

    /// Add an `Authorization` header, for example the result of
    /// [`basic_auth`](crate::encode::basic_auth) or
    /// [`bearer_auth`](crate::encode::bearer_auth).
    #[must_use]
    pub fn authorization(mut self, value: &'a str) -> Self {
        self.push(HttpHeader::authorization(value));
        self
    }

    /// Set the body without touching the headers.
    #[must_use]
    pub const fn body(mut self, body: &'a [u8]) -> Self {
        self.body = Some(body);
        self
    }

    /// Set a JSON body and add `Content-Type: application/json`.
    #[must_use]
    pub fn json(mut self, body: &'a str) -> Self {
        self.push(HttpHeader::content_type(mime_types::JSON));
        self.body = Some(body.as_bytes());
        self
    }

    /// Set an `application/x-www-form-urlencoded` body and add the matching
    /// `Content-Type`. The body must already be encoded; see
    /// [`QueryBuilder`](crate::encode::QueryBuilder).
    #[must_use]
    pub fn form(mut self, body: &'a [u8]) -> Self {
        self.push(HttpHeader::content_type(mime_types::FORM));
        self.body = Some(body);
        self
    }

    fn push(&mut self, header: HttpHeader<'a>) {
        if self.headers.push(header).is_err() {
            self.overflow = true;
        }
    }

    /// The request method.
    #[must_use]
    pub const fn method(&self) -> HttpMethod {
        self.method
    }

    /// The headers added so far.
    #[must_use]
    pub fn headers(&self) -> &[HttpHeader<'a>] {
        &self.headers
    }

    /// The request body, if one was set.
    #[must_use]
    pub const fn body_bytes(&self) -> Option<&'a [u8]> {
        self.body
    }

    /// Whether every header fit.
    ///
    /// # Errors
    ///
    /// Returns [`Error::BufferOverflow`] if more than [`MAX_HEADERS`] headers
    /// were added.
    pub const fn check(&self) -> Result<(), Error> {
        if self.overflow {
            Err(Error::BufferOverflow)
        } else {
            Ok(())
        }
    }
}
