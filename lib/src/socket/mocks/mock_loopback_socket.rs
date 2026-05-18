pub use crate::socket::mocks::error::MockStreamError;
use crate::socket::{SocketReadWith, SocketWriteWith};
use embedded_io_async::{ErrorType, Read, Write};
use ringbuf::{StaticRb, traits::*};
extern crate std;

/// Mock loopback socket for testing purposes.
pub struct MockLoopbackSocket<const BUFFER_SIZE: usize> {
    rb: StaticRb<u8, BUFFER_SIZE>,
}

impl<const BUFFER_SIZE: usize> MockLoopbackSocket<BUFFER_SIZE> {
    /// Creates a new `MockLoopbackSocket` with an empty buffer.
    pub fn new() -> Self {
        Self {
            rb: StaticRb::default(),
        }
    }
}

impl<const BUFFER_SIZE: usize> Default for MockLoopbackSocket<BUFFER_SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const BUFFER_SIZE: usize> ErrorType for MockLoopbackSocket<BUFFER_SIZE> {
    type Error = MockStreamError;
}

impl<const BUFFER_SIZE: usize> Read for MockLoopbackSocket<BUFFER_SIZE> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, MockStreamError> {
        Ok(self.rb.pop_slice(buf))
    }
}

impl<const BUFFER_SIZE: usize> Write for MockLoopbackSocket<BUFFER_SIZE> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, MockStreamError> {
        Ok(self.rb.push_slice(buf))
    }
}

impl<const BUFFER_SIZE: usize> SocketWriteWith for MockLoopbackSocket<BUFFER_SIZE> {
    async fn write_with<F, R>(&mut self, f: F) -> Result<R, MockStreamError>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        let mut temp_buf = [0u8; BUFFER_SIZE];
        let (written_size, result) = f(&mut temp_buf);
        assert!(written_size <= BUFFER_SIZE, "Wrote more bytes than available in buffer");
        self.rb.push_slice(&temp_buf[..written_size]);
        Ok(result)
    }
}

impl<const BUFFER_SIZE: usize> SocketReadWith for MockLoopbackSocket<BUFFER_SIZE> {
    async fn read_with<F, R>(&mut self, f: F) -> Result<R, MockStreamError>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        // Fill a temporary buffer from the ring buffer, call the provided closure,
        // and then skip the read bytes from the ring buffer.
        let mut temp_buf = [0u8; BUFFER_SIZE];
        let available_bytes = self.rb.peek_slice(&mut temp_buf);
        let (read_size, result) = f(&mut temp_buf[..available_bytes]);
        assert!(read_size <= available_bytes, "Read more bytes than available in buffer");
        let skipped = self.rb.skip(read_size);
        assert!(skipped == read_size, "Read more bytes than available in buffer");

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_basic_write_and_read() {
        let mut socket = MockLoopbackSocket::<64>::new();

        // Write some data
        let data = b"hello world";
        let written = socket.write(data).await.unwrap();
        assert_eq!(written, data.len());

        // Read it back
        let mut buf = [0u8; 64];
        let read = socket.read(&mut buf).await.unwrap();
        assert_eq!(read, data.len());
        assert_eq!(&buf[..read], data);
    }

    #[tokio::test]
    async fn test_write_with_functionality() {
        let mut socket = MockLoopbackSocket::<32>::new();

        let result = socket
            .write_with(|buf| {
                buf[0..5].copy_from_slice(b"test1");
                (5, 42)
            })
            .await
            .unwrap();

        assert_eq!(result, 42);

        let mut read_buf = [0u8; 10];
        let read = socket.read(&mut read_buf).await.unwrap();
        assert_eq!(read, 5);
        assert_eq!(&read_buf[..5], b"test1");
    }

    #[tokio::test]
    async fn test_read_with_functionality() {
        const TEST_STR: &str = "testing";
        let mut socket = MockLoopbackSocket::<32>::new();

        // First write some data
        socket.write(TEST_STR.as_bytes()).await.unwrap();
        let result = socket
            .read_with(|buf| {
                assert_eq!(buf.len(), TEST_STR.chars().count());
                let data_str = std::str::from_utf8(buf).unwrap();
                (buf.len(), data_str.to_string())
            })
            .await
            .unwrap();

        assert_eq!(result, "testing");

        // Buffer should be empty now
        let mut buf = [0u8; 10];
        let read = socket.read(&mut buf).await.unwrap();
        assert_eq!(read, 0);
    }

    #[tokio::test]
    async fn test_buffer_overflow_behavior() {
        let mut socket = MockLoopbackSocket::<8>::new();

        // Write more than buffer capacity
        let large_data = b"this is longer than 8 bytes";
        let written = socket.write(large_data).await.unwrap();
        assert!(written <= 8); // Should only write what fits

        let mut buf = [0u8; 32];
        let read = socket.read(&mut buf).await.unwrap();
        assert_eq!(read, written);
    }

    #[tokio::test]
    async fn test_multiple_write_read_cycles() {
        let mut socket = MockLoopbackSocket::<16>::new();

        for i in 0..3 {
            let data = format!("msg{}", i);
            let written = socket.write(data.as_bytes()).await.unwrap();
            assert_eq!(written, data.len());

            let mut buf = [0u8; 16];
            let read = socket.read(&mut buf).await.unwrap();
            assert_eq!(read, data.len());
            assert_eq!(&buf[..read], data.as_bytes());
        }
    }

    #[tokio::test]
    async fn test_partial_reads() {
        let mut socket = MockLoopbackSocket::<32>::new();

        // Write some data
        socket.write(b"hello world test").await.unwrap();

        // Read in smaller chunks
        let mut buf1 = [0u8; 5];
        let read1 = socket.read(&mut buf1).await.unwrap();
        assert_eq!(read1, 5);
        assert_eq!(&buf1, b"hello");

        let mut buf2 = [0u8; 6];
        let read2 = socket.read(&mut buf2).await.unwrap();
        assert_eq!(read2, 6);
        assert_eq!(&buf2, b" world");

        let mut buf3 = [0u8; 10];
        let read3 = socket.read(&mut buf3).await.unwrap();
        assert_eq!(read3, 5);
        assert_eq!(&buf3[..5], b" test");
    }

    #[tokio::test]
    async fn test_empty_buffer_operations() {
        let mut socket = MockLoopbackSocket::<16>::new();

        // Read from empty buffer
        let mut buf = [0u8; 10];
        let read = socket.read(&mut buf).await.unwrap();
        assert_eq!(read, 0);

        // Write empty slice
        let written = socket.write(&[]).await.unwrap();
        assert_eq!(written, 0);

        // Read should still return 0
        let read = socket.read(&mut buf).await.unwrap();
        assert_eq!(read, 0);
    }
}
