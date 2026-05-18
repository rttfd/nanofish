pub use crate::abstarct_socket::mocks::error::MockReadError;
pub use crate::abstarct_socket::socket::SocketReadWith;

use heapless::Vec;

/// A dummy multipart read stream for testing purposes
/// The stream simulates reading from multiple portions (buffers) sequentially like a
/// concatenated stream.
pub struct MockMultipartReadStream<'a, const N: usize> {
    multipart_buffer: Vec<&'a mut [u8], N>,
    part: usize,
    position: usize,
}

impl<'a, const N: usize> MockMultipartReadStream<'a, N> {
    /// Create a new DummyMultipartReadStream with the given multipart buffer
    pub fn new(multipart_buffer: [&'a mut [u8]; N]) -> Self {
        let mut buffer = Vec::new();
        for part in multipart_buffer.into_iter() {
            buffer.push(part).unwrap();
        }
        Self {
            multipart_buffer: buffer,
            part: 0,
            position: 0,
        }
    }

    /// Create a new DummyMultipartReadStream from an iterable collection of mutable byte slices
    pub fn from_collection<I: IntoIterator<Item = &'a mut [u8]>>(iter: I) -> Self {
        Self::from_iter(iter.into_iter())
    }

    //TODO: Impl FromIterator<> for MockMultipartReadStream to allow direct creation from iterators without needing a separate method.
    /// Create a new DummyMultipartReadStream from an iterator of mutable byte slices
    pub fn from_iter<I: Iterator<Item = &'a mut [u8]>>(iter: I) -> Self {
        let mut buffer = Vec::new();
        for part in iter {
            buffer.push(part).unwrap();
        }
        Self {
            multipart_buffer: buffer,
            part: 0,
            position: 0,
        }
    }
}

impl<'a, const N: usize> SocketReadWith for MockMultipartReadStream<'a, N> {
    async fn read_with<F, R>(&mut self, f: F) -> Result<R, Self::Error>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        if self.part >= self.multipart_buffer.len() {
            return Err(MockReadError::ConnectionReset);
        }

        if self.position >= self.multipart_buffer[self.part].len() {
            self.part += 1;
            self.position = 0;
            if self.part >= self.multipart_buffer.len() {
                return Err(MockReadError::ConnectionReset);
            }
        }

        let data = &mut self.multipart_buffer[self.part][self.position..];
        let (read_bytes, result) = f(data);
        self.position += read_bytes;
        Ok(result)
    }
}

mod embedded_io_impls {
    use super::*;
    impl<'a, const N: usize> embedded_io_async::ErrorType for MockMultipartReadStream<'a, N> {
        type Error = MockReadError;
    }

    impl<'a, const N: usize> embedded_io_async::Read for MockMultipartReadStream<'a, N> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            if buf.is_empty() || self.part >= self.multipart_buffer.len() {
                // EOF reached
                return Ok(0);
            }

            let to_read = core::cmp::min(buf.len(), self.multipart_buffer[self.part].len() - self.position);

            buf[..to_read].copy_from_slice(&self.multipart_buffer[self.part][self.position..self.position + to_read]);
            self.position += to_read;

            if self.position >= self.multipart_buffer[self.part].len() {
                self.part += 1;
                self.position = 0;
            }

            Ok(to_read)
        }
    }

    impl<'a, const N: usize> embedded_io_async::ReadReady for MockMultipartReadStream<'a, N> {
        fn read_ready(&mut self) -> Result<bool, Self::Error> {
            Ok(self.part < self.multipart_buffer.len() && self.position < self.multipart_buffer[self.part].len())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use embedded_io_async::{Read, ReadReady};
    extern crate std;

    #[test]
    fn test_new() {
        const N: usize = 3;
        let array: [&mut [u8]; N] = [&mut [1, 2, 3], &mut [4, 5], &mut [6, 7, 8, 9]];
        let stream = MockMultipartReadStream::<'_, N>::new(array);
        assert_eq!(stream.multipart_buffer.len(), 3);
        assert_eq!(stream.multipart_buffer[0], &[1, 2, 3]);
        assert_eq!(stream.multipart_buffer[1], &[4, 5]);
        assert_eq!(stream.multipart_buffer[2], &[6, 7, 8, 9]);
    }

    #[tokio::test]
    async fn test_empty_multipart_buffer_is_empty() {
        let mut stream = MockMultipartReadStream::new([]);
        let mut buf = [0u8; 10];

        let result = stream.read(&mut buf).await.expect("Failed to read from stream");
        assert_eq!(result, 0);
    }

    #[tokio::test]
    async fn test_single_part_read() {
        let parts: [&mut [u8]; _] = [&mut [1, 2, 3, 4, 5]];
        let mut stream = MockMultipartReadStream::new(parts);
        let mut buf = [0u8; 10];

        let bytes_read = stream.read(&mut buf).await.unwrap();
        assert_eq!(bytes_read, 5);
        assert_eq!(&buf[..5], &[1, 2, 3, 4, 5]);

        // Second read should return 0 (EOF)
        let bytes_read = stream.read(&mut buf).await.unwrap();
        assert_eq!(bytes_read, 0);
    }

    #[tokio::test]
    async fn test_multipart_sequential_read() {
        let parts: [&mut [u8]; _] = [&mut [1, 2, 3], &mut [4, 5], &mut [6, 7, 8, 9]];
        let mut stream = MockMultipartReadStream::new(parts);
        let mut buf = [0u8; 2];

        // Read first portion partially: [1, 2]
        let bytes_read = stream.read(&mut buf).await.unwrap();
        assert_eq!(bytes_read, 2);
        assert_eq!(&buf, &[1, 2]);

        // Continue reading next portion of first part: [3]
        let bytes_read = stream.read(&mut buf).await.unwrap();
        assert_eq!(bytes_read, 1);
        assert_eq!(&buf[..bytes_read], &[3]);

        // Read second part: [4, 5]
        let bytes_read = stream.read(&mut buf).await.unwrap();
        assert_eq!(bytes_read, 2);
        assert_eq!(&buf, &[4, 5]);

        // Read the first portion of third part: [6, 7]
        let bytes_read = stream.read(&mut buf).await.unwrap();
        assert_eq!(bytes_read, 2);
        assert_eq!(&buf, &[6, 7]);

        // Read the second portion of third part: [8, 9]
        let bytes_read = stream.read(&mut buf).await.unwrap();
        assert_eq!(bytes_read, 2);
        assert_eq!(&buf, &[8, 9]);

        // EOF
        let bytes_read = stream.read(&mut buf).await.unwrap();
        assert_eq!(bytes_read, 0);
    }

    #[tokio::test]
    async fn test_read_with_function() {
        let parts: [&mut [u8]; _] = [&mut [1, 2, 3], &mut [4, 5], &mut [6, 7, 8, 9]];
        let mut stream = MockMultipartReadStream::new(parts);

        let result = stream
            .read_with(|data| {
                let sum: u32 = data.iter().take(3).map(|&x| x as u32).sum();
                (3, sum)
            })
            .await
            .unwrap();

        assert_eq!(result, 6); // 1 + 2 + 3
    }

    #[tokio::test]
    async fn test_read_with_empty_buffer() {
        let mut stream = MockMultipartReadStream::new([]);

        let result = stream.read_with(|_data| (0, 42)).await;
        assert!(matches!(result, Err(MockReadError::ConnectionReset)));
    }

    #[tokio::test]
    async fn test_buffer_boundary_transitions() {
        let parts: [&mut [u8]; _] = [&mut [1], &mut [2], &mut [3]];
        let mut stream = MockMultipartReadStream::new(parts);
        let mut buf = [0u8; 1];

        // Read each part
        for expected in [1, 2, 3] {
            let bytes_read = stream.read(&mut buf).await.unwrap();
            assert_eq!(bytes_read, 1);
            assert_eq!(buf[0], expected);
        }

        // EOF
        let bytes_read = stream.read(&mut buf).await.unwrap();
        assert_eq!(bytes_read, 0);
    }

    #[tokio::test]
    async fn test_read_larger_than_available() {
        let parts: [&mut [u8]; _] = [&mut [1, 2, 3]];
        let mut stream = MockMultipartReadStream::new(parts);
        let mut buf = [0u8; 10];

        let bytes_read = stream.read(&mut buf).await.unwrap();
        assert_eq!(bytes_read, 3);
        assert_eq!(&buf[..3], &[1, 2, 3]);
    }

    #[tokio::test]
    async fn test_empty_buffer_read() {
        let parts: [&mut [u8]; _] = [&mut [1, 2, 3]];
        let mut stream = MockMultipartReadStream::new(parts);
        let mut buf = [];

        let bytes_read = stream.read(&mut buf).await.unwrap();
        assert_eq!(bytes_read, 0);
    }

    #[tokio::test]
    async fn test_read_ready() {
        let parts: [&mut [u8]; _] = [&mut [1, 2, 3]];
        let mut stream = MockMultipartReadStream::new(parts);
        let is_ready = stream.read_ready().unwrap();
        assert!(is_ready);
        // Read all data
        let mut buf = [0u8; 3];
        stream.read(&mut buf).await.unwrap();
        let is_ready_after_eof = stream.read_ready().unwrap();
        assert!(!is_ready_after_eof);
    }
}
