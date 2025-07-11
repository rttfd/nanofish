use embassy_time::Duration;

/// Options for configuring the HTTP client
#[derive(Clone, Copy, Debug, PartialEq)]
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

/// Buffer sizes for read and write operations
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BufferSize {
    /// Size of the read buffer
    pub read: usize,
    /// Size of the write buffer
    pub write: usize,
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

#[cfg(test)]
mod tests {
    use super::*;
    use embassy_time::Duration;

    #[test]
    fn test_default_options() {
        let opts = HttpClientOptions::default();
        assert_eq!(opts.max_retries, 5);
        assert_eq!(opts.socket_timeout, Duration::from_secs(60));
        assert_eq!(opts.retry_delay, Duration::from_millis(200));
        assert_eq!(opts.socket_close_delay, Duration::from_millis(100));
    }

    #[test]
    fn test_custom_options() {
        let opts = HttpClientOptions {
            max_retries: 2,
            socket_timeout: Duration::from_secs(10),
            retry_delay: Duration::from_millis(50),
            socket_close_delay: Duration::from_millis(20),
        };
        assert_eq!(opts.max_retries, 2);
        assert_eq!(opts.socket_timeout, Duration::from_secs(10));
        assert_eq!(opts.retry_delay, Duration::from_millis(50));
        assert_eq!(opts.socket_close_delay, Duration::from_millis(20));
    }
}
