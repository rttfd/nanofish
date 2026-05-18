pub use crate::abstarct_socket::mocks::error::MockReadError;
pub use crate::abstarct_socket::socket::SocketReadWith;

/// A dummy read stream for testing purposes
pub struct MockReadStream<'a> {
    buffer: &'a mut [u8],
    position: usize,
}

impl<'a> MockReadStream<'a> {
    /// Create a new `MockReadStream` with the given buffer. The stream will read from the buffer starting at position 0.
    pub fn new(buffer: &'a mut [u8]) -> Self {
        Self { buffer, position: 0 }
    }
}

impl<'a> SocketReadWith for MockReadStream<'a> {
    async fn read_with<F, R>(&mut self, f: F) -> Result<R, Self::Error>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        if self.position >= self.buffer.len() {
            return Err(MockReadError::ConnectionReset);
        }

        let data = &mut self.buffer[self.position..];
        let (read_bytes, result) = f(data);
        assert!(read_bytes <= data.len(), "Read more bytes than available in buffer");
        self.position += read_bytes;
        Ok(result)
    }
}

mod embedded_io_impls {
    use super::*;
    impl<'d> embedded_io_async::ErrorType for MockReadStream<'d> {
        type Error = MockReadError;
    }

    impl<'d> embedded_io_async::Read for MockReadStream<'d> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            if self.position >= self.buffer.len() || buf.is_empty() {
                // EOF reached
                return Ok(0);
            }

            let to_read = core::cmp::min(buf.len(), self.buffer.len() - self.position);
            buf[..to_read].copy_from_slice(&self.buffer[self.position..self.position + to_read]);
            self.position += to_read;
            Ok(to_read)
        }
    }

    impl<'d> embedded_io_async::ReadReady for MockReadStream<'d> {
        fn read_ready(&mut self) -> Result<bool, Self::Error> {
            Ok(self.position < self.buffer.len())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use embedded_io_async::{Read, ReadReady};

    #[tokio::test]
    async fn test_new() {
        let mut buffer = vec![1, 2, 3, 4, 5];
        let stream = MockReadStream::new(&mut buffer);
        assert_eq!(stream.position, 0);
    }

    #[tokio::test]
    async fn test_read_empty_buffer() {
        let mut buffer = vec![];
        let mut stream = MockReadStream::new(&mut buffer);
        let mut buf = [0u8; 10];

        let result = stream.read(&mut buf).await.unwrap();
        assert_eq!(result, 0);
    }

    #[tokio::test]
    async fn test_read_partial() {
        let mut buffer = vec![1, 2, 3, 4, 5];
        let mut stream = MockReadStream::new(&mut buffer);
        let mut buf = [0u8; 3];

        let result = stream.read(&mut buf).await.unwrap();
        assert_eq!(result, 3);
        assert_eq!(buf, [1, 2, 3]);
    }

    #[tokio::test]
    async fn test_read_full_buffer() {
        let mut buffer = vec![1, 2, 3];
        let mut stream = MockReadStream::new(&mut buffer);
        let mut buf = [0u8; 5];

        let result = stream.read(&mut buf).await.unwrap();
        assert_eq!(result, 3);
        assert_eq!(buf[..3], [1, 2, 3]);
    }

    #[tokio::test]
    async fn test_read_multiple_times() {
        let mut buffer = vec![1, 2, 3, 4, 5];
        let mut stream = MockReadStream::new(&mut buffer);
        let mut buf = [0u8; 2];

        let result1 = stream.read(&mut buf).await.unwrap();
        assert_eq!(result1, 2);
        assert_eq!(buf, [1, 2]);

        let result2 = stream.read(&mut buf).await.unwrap();
        assert_eq!(result2, 2);
        assert_eq!(buf, [3, 4]);

        let result3 = stream.read(&mut buf).await.unwrap();
        assert_eq!(result3, 1);
        assert_eq!(buf[0], 5);
    }

    #[tokio::test]
    async fn test_read_at_eof() {
        let mut buffer = vec![1, 2, 3];
        let mut stream = MockReadStream::new(&mut buffer);
        let mut buf = [0u8; 5];

        // Read all data
        stream.read(&mut buf).await.unwrap();

        // Try to read again at EOF
        let result = stream.read(&mut buf).await.unwrap();
        assert_eq!(result, 0);
    }

    #[tokio::test]
    async fn test_read_with_success() {
        let mut buffer = vec![1, 2, 3, 4, 5];
        let mut stream = MockReadStream::new(&mut buffer);

        let result = stream
            .read_with(|data| {
                assert_eq!(data, &[1, 2, 3, 4, 5]);
                (3, "success")
            })
            .await
            .unwrap();

        assert_eq!(result, "success");
        assert_eq!(stream.position, 3);
    }

    #[tokio::test]
    async fn test_read_with_at_eof() {
        let mut buffer = vec![1, 2, 3];
        let mut stream = MockReadStream::new(&mut buffer);
        stream.position = 3; // Set to EOF

        let result = stream.read_with(|_| (0, "test")).await;
        assert!(matches!(result, Err(MockReadError::ConnectionReset)));
    }

    #[tokio::test]
    async fn test_read_with_partial_read() {
        let mut buffer = vec![1, 2, 3, 4, 5];
        let mut stream = MockReadStream::new(&mut buffer);
        stream.position = 2; // Start from position 2

        let result = stream
            .read_with(|data| {
                assert_eq!(data, &[3, 4, 5]);
                (2, 42)
            })
            .await
            .unwrap();

        assert_eq!(result, 42);
        assert_eq!(stream.position, 4);
    }

    #[tokio::test]
    async fn test_read_ready() {
        let mut buffer = vec![1, 2, 3];
        let mut stream = MockReadStream::new(&mut buffer);
        let is_ready = stream.read_ready().unwrap();
        assert!(is_ready);

        // Read all data
        let mut buf = [0u8; 3];
        stream.read(&mut buf).await.unwrap();
        let is_ready_after_eof = stream.read_ready().unwrap();
        assert!(!is_ready_after_eof);
    }
}
