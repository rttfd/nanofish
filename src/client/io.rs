use crate::{
    codec,
    error::Error,
    header::HttpHeader,
    method::HttpMethod,
    options::HttpClientOptions,
    protocol::{DEFAULT_HTTP_PORT, DEFAULT_HTTPS_PORT},
    response::HttpResponse,
};
use embedded_io_async::{Read, Write};

const DEFAULT_REQUEST_SIZE: usize = 1024;
const SMALL_REQUEST_SIZE: usize = 1024;

/// Parsed HTTP endpoint metadata.
pub struct HttpEndpoint<'a> {
    /// URL scheme, either `http` or `https`.
    pub scheme: &'a str,
    /// Hostname without port.
    pub host: &'a str,
    /// Explicit or default port.
    pub port: u16,
    /// Request path, defaulting to `/`.
    pub path: &'a str,
}

/// Parse an HTTP or HTTPS URL into endpoint metadata.
///
/// # Errors
///
/// Returns [`Error::InvalidUrl`] if the endpoint does not start with `http://` or `https://`.
pub fn parse_endpoint(endpoint: &str) -> Result<HttpEndpoint<'_>, Error> {
    let (scheme, host_port) = if let Some(rest) = endpoint.strip_prefix("http://") {
        ("http", rest)
    } else if let Some(rest) = endpoint.strip_prefix("https://") {
        ("https", rest)
    } else {
        return Err(Error::InvalidUrl);
    };

    let host = host_port.split('/').next().ok_or(Error::InvalidUrl)?;
    let path = &host_port[host.len()..];
    let path = if path.is_empty() { "/" } else { path };

    let default_port = if scheme == "https" {
        DEFAULT_HTTPS_PORT
    } else {
        DEFAULT_HTTP_PORT
    };
    let (host, port) = host.rfind(':').map_or((host, default_port), |colon_pos| {
        host[colon_pos + 1..]
            .parse::<u16>()
            .map_or((host, default_port), |port| (&host[..colon_pos], port))
    });

    Ok(HttpEndpoint {
        scheme,
        host,
        port,
        path,
    })
}

/// Request metadata for [`HttpClient`].
pub struct HttpClientRequest<'a> {
    /// HTTP method to send.
    pub method: HttpMethod,
    /// Host value used for the HTTP `Host` header.
    pub host: &'a str,
    /// Request target, for example `/`, `/api`, or `/api?limit=10`.
    pub path: &'a str,
    /// Additional request headers.
    pub headers: &'a [HttpHeader<'a>],
    /// Optional request body.
    pub body: Option<&'a [u8]>,
}

/// Transport-generic HTTP client for already-connected streams.
pub struct HttpClient<const RQ: usize = DEFAULT_REQUEST_SIZE> {
    options: HttpClientOptions,
}

impl HttpClient<DEFAULT_REQUEST_SIZE> {
    /// Create a new transport-generic client with default options and buffer sizes.
    #[must_use]
    pub fn new() -> Self {
        Self {
            options: HttpClientOptions::default(),
        }
    }
}

impl<const RQ: usize> HttpClient<RQ> {
    /// Create a new transport-generic client with custom options.
    #[must_use]
    pub const fn with_options(options: HttpClientOptions) -> Self {
        Self { options }
    }

    /// Send one HTTP request over an already-connected stream.
    ///
    /// # Errors
    ///
    /// Returns an error if request construction fails, stream IO fails, no
    /// response is received, or the response cannot be parsed.
    pub async fn request<'b, S>(
        &self,
        stream: &mut S,
        request: HttpClientRequest<'_>,
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error>
    where
        S: Read + Write,
    {
        let http_request = codec::build_req::<RQ>(
            request.method,
            request.host,
            request.path,
            request.headers,
            request.body,
        )?;

        stream
            .write_all(http_request.as_bytes())
            .await
            .map_err(|_| Error::TcpError)?;

        if let Some(body_data) = request.body {
            stream
                .write_all(body_data)
                .await
                .map_err(|_| Error::TcpError)?;
        }

        stream.flush().await.map_err(|_| Error::TcpError)?;

        let total_read = read_response(stream, response_buffer, self.options.max_retries).await?;
        let total_read = codec::dechunk(response_buffer, total_read)?;
        let response = codec::parse_resp(&response_buffer[..total_read])?;
        Ok((response, total_read))
    }

    /// Convenience method for making a GET request over an already-connected stream.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpClient::request`].
    pub async fn get<'b, S>(
        &self,
        stream: &mut S,
        host: &str,
        path: &str,
        headers: &[HttpHeader<'_>],
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error>
    where
        S: Read + Write,
    {
        self.request(
            stream,
            HttpClientRequest {
                method: HttpMethod::GET,
                host,
                path,
                headers,
                body: None,
            },
            response_buffer,
        )
        .await
    }

    /// Convenience method for making a POST request over an already-connected stream.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`HttpClient::request`].
    pub async fn post<'b, S>(
        &self,
        stream: &mut S,
        host: &str,
        path: &str,
        headers: &[HttpHeader<'_>],
        body: &[u8],
        response_buffer: &'b mut [u8],
    ) -> Result<(HttpResponse<'b>, usize), Error>
    where
        S: Read + Write,
    {
        self.request(
            stream,
            HttpClientRequest {
                method: HttpMethod::POST,
                host,
                path,
                headers,
                body: Some(body),
            },
            response_buffer,
        )
        .await
    }
}

impl Default for HttpClient<DEFAULT_REQUEST_SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

/// Type alias for `HttpClient` with the default request buffer size.
pub type DefaultHttpClient = HttpClient<DEFAULT_REQUEST_SIZE>;

/// Type alias for `HttpClient` with a smaller request buffer size.
pub type SmallHttpClient = HttpClient<SMALL_REQUEST_SIZE>;

/// Transport-generic HTTPS client for already-connected streams.
#[cfg(feature = "tls")]
pub struct HttpTlsClient<
    const RQ: usize = DEFAULT_REQUEST_SIZE,
    const TLS_READ: usize = 4096,
    const TLS_WRITE: usize = 4096,
> {
    options: HttpClientOptions,
}

#[cfg(feature = "tls")]
impl HttpTlsClient<DEFAULT_REQUEST_SIZE, 4096, 4096> {
    /// Create a new transport-generic TLS client with default buffer sizes.
    #[must_use]
    pub fn new() -> Self {
        Self {
            options: HttpClientOptions::default(),
        }
    }
}

#[cfg(feature = "tls")]
impl<const RQ: usize, const TLS_READ: usize, const TLS_WRITE: usize>
    HttpTlsClient<RQ, TLS_READ, TLS_WRITE>
{
    /// Create a new transport-generic TLS client with custom options.
    #[must_use]
    pub const fn with_options(options: HttpClientOptions) -> Self {
        Self { options }
    }

    /// Send one HTTPS request over an already-connected stream.
    ///
    /// # Errors
    ///
    /// Returns an error if TLS handshake/IO fails, request construction fails,
    /// no response is received, or the response cannot be parsed.
    #[expect(clippy::future_not_send)]
    pub async fn request<'b, S, RNG>(
        &self,
        stream: S,
        server_name: &str,
        request: HttpClientRequest<'_>,
        response_buffer: &'b mut [u8],
        rng: RNG,
    ) -> Result<(HttpResponse<'b>, usize), Error>
    where
        S: Read + Write,
        RNG: embedded_tls::CryptoRngCore,
    {
        let tls_config = embedded_tls::TlsConfig::new().with_server_name(server_name);
        let mut read_record_buffer = [0; TLS_READ];
        let mut write_record_buffer = [0; TLS_WRITE];
        let mut tls = embedded_tls::TlsConnection::new(
            stream,
            &mut read_record_buffer,
            &mut write_record_buffer,
        );

        tls.open(embedded_tls::TlsContext::new(
            &tls_config,
            embedded_tls::UnsecureProvider::new::<embedded_tls::Aes128GcmSha256>(rng),
        ))
        .await?;

        let client = HttpClient::<RQ>::with_options(self.options);
        let result = client.request(&mut tls, request, response_buffer).await;
        let _ = tls.close().await;
        result
    }
}

#[cfg(feature = "tls")]
impl Default for HttpTlsClient<DEFAULT_REQUEST_SIZE, 4096, 4096> {
    fn default() -> Self {
        Self::new()
    }
}

/// Type alias for `HttpTlsClient` with default request and TLS buffer sizes.
#[cfg(feature = "tls")]
pub type DefaultHttpTlsClient = HttpTlsClient<DEFAULT_REQUEST_SIZE, 4096, 4096>;

/// Type alias for `HttpTlsClient` with smaller request and TLS buffer sizes.
#[cfg(feature = "tls")]
pub type SmallHttpTlsClient = HttpTlsClient<SMALL_REQUEST_SIZE, 1024, 1024>;

async fn read_response<S>(
    stream: &mut S,
    response_buffer: &mut [u8],
    max_retries: usize,
) -> Result<usize, Error>
where
    S: Read,
{
    let mut total_read = 0;
    let mut retries = max_retries;

    while total_read < response_buffer.len() && retries > 0 {
        match stream.read(&mut response_buffer[total_read..]).await {
            Ok(0) => break,
            Ok(n) => {
                total_read += n;
                if codec::complete(&response_buffer[..total_read]) {
                    break;
                }
            }
            Err(_) => {
                retries -= 1;
                if retries == 0 {
                    return Err(Error::TcpError);
                }
            }
        }
    }

    if total_read == 0 {
        return Err(Error::NoResponse);
    }

    Ok(total_read)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ResponseBody, StatusCode};
    use core::convert::Infallible;
    use embedded_io_async::ErrorType;
    use heapless::Vec;

    struct MockStream<const IN: usize, const OUT: usize> {
        input: Vec<u8, IN>,
        output: Vec<u8, OUT>,
        read_pos: usize,
    }

    impl<const IN: usize, const OUT: usize> MockStream<IN, OUT> {
        fn new(input: &[u8]) -> Self {
            Self {
                input: Vec::from_slice(input).unwrap(),
                output: Vec::new(),
                read_pos: 0,
            }
        }
    }

    impl<const IN: usize, const OUT: usize> ErrorType for MockStream<IN, OUT> {
        type Error = Infallible;
    }

    impl<const IN: usize, const OUT: usize> Read for MockStream<IN, OUT> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            if self.read_pos >= self.input.len() {
                return Ok(0);
            }
            let n = buf.len().min(self.input.len() - self.read_pos);
            buf[..n].copy_from_slice(&self.input[self.read_pos..self.read_pos + n]);
            self.read_pos += n;
            Ok(n)
        }
    }

    impl<const IN: usize, const OUT: usize> Write for MockStream<IN, OUT> {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            self.output.extend_from_slice(buf).unwrap();
            Ok(buf.len())
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[test]
    fn test_io_client_get() {
        let response =
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nhello";
        let mut stream = MockStream::<128, 256>::new(response);
        let client = HttpClient::new();
        let mut buffer = [0; 128];

        let (response, _) = futures_lite::future::block_on(client.get(
            &mut stream,
            "example.com",
            "/hello",
            &[],
            &mut buffer,
        ))
        .unwrap();

        assert_eq!(response.status_code, StatusCode::Ok);
        assert_eq!(response.body.as_str(), Some("hello"));
        let request = core::str::from_utf8(&stream.output).unwrap();
        assert!(request.starts_with("GET /hello HTTP/1.1\r\nHost: example.com\r\n"));
    }

    #[test]
    fn test_io_client_binary_response() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 3\r\n\r\n\x01\x02\x03";
        let mut stream = MockStream::<128, 256>::new(response);
        let client = HttpClient::new();
        let mut buffer = [0; 128];

        let (response, _) = futures_lite::future::block_on(client.get(
            &mut stream,
            "example.com",
            "/bin",
            &[],
            &mut buffer,
        ))
        .unwrap();

        assert_eq!(response.body, ResponseBody::Binary(&[1, 2, 3]));
    }
}
