/// HTTP Header struct for representing a single header
///
/// This struct represents a single HTTP header with a name and value.
/// Headers are used to pass additional information about the request or response.
#[derive(Clone, Debug)]
pub struct HttpHeader<'a> {
    /// The name of the header (e.g., "Content-Type", "Authorization")
    pub name: &'a str,
    /// The value of the header (e.g., "application/json", "Bearer token123")
    pub value: &'a str,
}
