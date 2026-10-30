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
    /// Overall deadline for one request, from DNS resolution to the end of the
    /// response, or `None` for no limit (the default).
    ///
    /// `socket_timeout` only bounds idle time on the socket, so a server that
    /// trickles a byte every few seconds can hold a request open indefinitely.
    /// Applied by the Embassy-backed clients, which fail with
    /// [`Error::Timeout`](nanofish::Error::Timeout); transport-generic clients
    /// have no timer, so wrap their futures in the timeout of your runtime.
    pub total_timeout: Option<TimeoutDuration>,
    /// Maximum number of redirects (`301`, `302`, `303`, `307`, `308`) the
    /// Embassy-backed clients follow before failing with
    /// [`Error::InvalidResponse`](nanofish::Error::InvalidResponse). `0` (the
    /// default) returns redirect responses to the caller untouched.
    ///
    /// Each hop opens a new connection and reuses the response buffer, so the
    /// final response replaces the redirect responses in it. `Authorization`,
    /// `Cookie` and `Proxy-Authorization` headers are dropped when a redirect
    /// leaves the original origin, a redirect from `https` to `http` is
    /// refused, and a `POST` answered by `301`, `302` or `303` is repeated as
    /// a `GET` without its body.
    pub max_redirects: usize,
    /// Delay between read retry attempts.
    ///
    /// Honored by request APIs that accept a retry delay (for example the
    /// Embassy-backed clients, which sleep between attempts). Transport-neutral
    /// APIs retry immediately unless given a delay via
    /// [`HttpClient::request_with_retry_delay`](nanofish::client::HttpClient::request_with_retry_delay).
    pub retry_delay: TimeoutDuration,
    /// Delay after closing a socket before proceeding
    pub socket_close_delay: TimeoutDuration,
}

impl Default for HttpClientOptions {
    fn default() -> Self {
        Self {
            max_retries: 5,
            socket_timeout: TimeoutDuration::from_secs(60),
            total_timeout: None,
            max_redirects: 0,
            retry_delay: TimeoutDuration::from_millis(200),
            socket_close_delay: TimeoutDuration::from_millis(100),
        }
    }
}

#[cfg(test)]
mod tests {
    use nanofish::options::*;

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
        assert_eq!(opts.total_timeout, None);
        assert_eq!(opts.max_redirects, 0);
        assert_eq!(opts.retry_delay, TimeoutDuration::from_millis(200));
        assert_eq!(opts.socket_close_delay, TimeoutDuration::from_millis(100));
    }

    #[test]
    fn test_custom_options() {
        let opts = HttpClientOptions {
            max_retries: 2,
            socket_timeout: TimeoutDuration::from_secs(10),
            total_timeout: Some(TimeoutDuration::from_secs(30)),
            max_redirects: 3,
            retry_delay: TimeoutDuration::from_millis(50),
            socket_close_delay: TimeoutDuration::from_millis(20),
        };
        assert_eq!(opts.max_retries, 2);
        assert_eq!(opts.socket_timeout, TimeoutDuration::from_secs(10));
        assert_eq!(opts.total_timeout, Some(TimeoutDuration::from_secs(30)));
        assert_eq!(opts.max_redirects, 3);
        assert_eq!(opts.retry_delay, TimeoutDuration::from_millis(50));
        assert_eq!(opts.socket_close_delay, TimeoutDuration::from_millis(20));
    }
}
