use embassy_time::Duration;

/// Options for configuring the HTTP client
pub struct HttpClientOptions {
    /// Maximum number of retries for read operations
    pub max_retries: usize,
    /// Timeout duration for socket operations
    pub socket_timeout: Duration,
    /// Delay between retry attempts
    pub retry_delay: Duration,
    /// Delay after closing a socket before proceeding
    pub socket_close_delay: Duration,
}

impl Default for HttpClientOptions {
    fn default() -> Self {
        Self {
            max_retries: 5,
            socket_timeout: Duration::from_secs(60),
            retry_delay: Duration::from_millis(200),
            socket_close_delay: Duration::from_millis(100),
        }
    }
}
