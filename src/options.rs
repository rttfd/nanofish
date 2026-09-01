/// A transport-neutral timeout duration used by client options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeoutDuration {
    millis: u64,
}

impl TimeoutDuration {
    /// Create a timeout duration from milliseconds.
    #[must_use]
    pub const fn from_millis(millis: u64) -> Self {
        Self { millis }
    }

    /// Create a timeout duration from seconds.
    #[must_use]
    pub const fn from_secs(secs: u64) -> Self {
        Self {
            millis: secs.saturating_mul(1_000),
        }
    }

    /// Return the duration as milliseconds.
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.millis
    }
}

/// Options for configuring the HTTP client
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpClientOptions {
    /// Maximum number of read attempts for response reads
    pub max_retries: usize,
    /// Timeout duration for socket operations
    pub socket_timeout: TimeoutDuration,
    /// Delay between read retry attempts.
    ///
    /// Honored by request APIs that accept a retry delay (for example the
    /// Embassy-backed clients, which sleep between attempts). Transport-neutral
    /// APIs retry immediately unless given a delay via
    /// [`HttpClient::request_with_retry_delay`](crate::client::HttpClient::request_with_retry_delay).
    pub retry_delay: TimeoutDuration,
    /// Delay after closing a socket before proceeding
    pub socket_close_delay: TimeoutDuration,
}

impl Default for HttpClientOptions {
    fn default() -> Self {
        Self {
            max_retries: 5,
            socket_timeout: TimeoutDuration::from_secs(60),
            retry_delay: TimeoutDuration::from_millis(200),
            socket_close_delay: TimeoutDuration::from_millis(100),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timeout_duration_constructors() {
        assert_eq!(TimeoutDuration::from_millis(123).as_millis(), 123);
        assert_eq!(TimeoutDuration::from_secs(2).as_millis(), 2_000);
    }

    #[test]
    fn test_default_options() {
        let opts = HttpClientOptions::default();
        assert_eq!(opts.max_retries, 5);
        assert_eq!(opts.socket_timeout, TimeoutDuration::from_secs(60));
        assert_eq!(opts.retry_delay, TimeoutDuration::from_millis(200));
        assert_eq!(opts.socket_close_delay, TimeoutDuration::from_millis(100));
    }

    #[test]
    fn test_custom_options() {
        let opts = HttpClientOptions {
            max_retries: 2,
            socket_timeout: TimeoutDuration::from_secs(10),
            retry_delay: TimeoutDuration::from_millis(50),
            socket_close_delay: TimeoutDuration::from_millis(20),
        };
        assert_eq!(opts.max_retries, 2);
        assert_eq!(opts.socket_timeout, TimeoutDuration::from_secs(10));
        assert_eq!(opts.retry_delay, TimeoutDuration::from_millis(50));
        assert_eq!(opts.socket_close_delay, TimeoutDuration::from_millis(20));
    }
}
