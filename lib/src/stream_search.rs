#![allow(dead_code)]
use crate::abstarct_socket::socket::SocketReadWith;
use crate::find_sequence::FindSequence;
use prefix_arena::{PrefixArena, StagingBuffer};

/// Error returned by the stream-reading helper methods in this module.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum StreamReadError<T> {
    /// The underlying stream read failed.
    SocketReadError(T),
    /// The allocator-backed output buffer was too small to hold the collected bytes.
    ReadBufferOverflow,
}

#[cfg(feature = "defmt")]
impl<T: core::fmt::Debug> defmt::Format for StreamReadError<T> {
    fn format(&self, fmt: defmt::Formatter) {
        match self {
            StreamReadError::SocketReadError(e) => {
                defmt::write!(fmt, "Socket read error: {:?}", defmt::Debug2Format(e))
            }
            StreamReadError::ReadBufferOverflow => defmt::write!(fmt, "Read buffer overflow"),
        }
    }
}

#[cfg(not(any(feature = "defmt", feature = "log")))]
impl<T> defmt_or_log::FormatOrDebug for StreamReadError<T> {}

/// Search and scan helpers for [`ReadWith`] streams.
///
/// The `seek_*` methods collect bytes into an arena-backed buffer and return the data
/// read up to the matched boundary. The `skip_*` methods advance the stream without
/// storing the skipped bytes.
pub trait StreamSearch: SocketReadWith {
    /// Seeks through the stream until `stop_condition` matches.
    ///
    /// Each chunk is passed to `stop_condition`. Returning `Some(len)` means that the
    /// stop condition was met in the current chunk and that only the first `len` bytes
    /// of that chunk should be appended to the output. Returning `None` continues
    /// reading.
    ///
    /// The returned slice contains every byte that was appended, including the bytes
    /// from the chunk that satisfied the stop condition.
    ///
    /// ## Errors
    /// - Returns [`StreamReadError::SocketReadError`] if reading from the stream fails.
    /// - Returns [`StreamReadError::ReadBufferOverflow`] if the allocator does not
    ///   have enough capacity for the collected bytes.
    fn seek_until<'alloc, 'buf, StopPredicate>(
        &mut self,
        allocator: &'alloc mut PrefixArena<'buf>,
        mut stop_condition: StopPredicate,
    ) -> impl core::future::Future<Output = Result<&'buf mut [u8], StreamReadError<Self::Error>>>
    where
        StopPredicate: FnMut(&mut [u8]) -> Option<usize>,
        'buf: 'alloc,
    {
        async move {
            let mut result = Ok(());

            let mut buffer = StagingBuffer::new(allocator);

            loop {
                let stop_triggered = self
                    .read_with(|mut chunk: &mut [u8]| {
                        let mut stopped = false;
                        if let Some(matched_len) = stop_condition(chunk) {
                            // SAFETY: matched_len is guaranteed to be <= chunk.len().
                            chunk = unsafe { chunk.split_at_mut_unchecked(matched_len).0 };
                            stopped = true;
                        }

                        let appended_len = buffer.extend_from_slice_capped(chunk);
                        if appended_len < chunk.len() {
                            result = Err(StreamReadError::ReadBufferOverflow);
                            stopped = true;
                        }

                        (appended_len, stopped)
                    })
                    .await
                    .map_err(StreamReadError::SocketReadError)?;

                if stop_triggered {
                    return result.map(|_| buffer.into_written_slice());
                }
            }
        }
    }

    /// Seeks through the stream until `stop_sequence` is found.
    ///
    /// The returned slice is stored in the provided allocator-backed buffer and
    /// includes `stop_sequence`.
    ///
    /// ## Errors
    /// - Returns [`StreamReadError::ReadBufferOverflow`] if the allocator does not
    ///   have enough capacity for the collected bytes.
    /// - Returns [`StreamReadError::SocketReadError`] if reading from the stream fails.
    fn seek_until_sequence<'alloc, 'buf>(
        &mut self,
        stop_sequence: &[u8],
        allocator: &'alloc mut PrefixArena<'buf>,
    ) -> impl core::future::Future<Output = Result<&'buf mut [u8], StreamReadError<Self::Error>>>
    where
        'buf: 'alloc,
    {
        async move {
            let mut finder = FindSequence::new(stop_sequence);
            self.seek_until(allocator, |chunk| finder.check_next_slice(chunk)).await
        }
    }

    /// Seeks through the stream until `stop_byte` is found.
    ///
    /// The returned slice is stored in the provided allocator-backed buffer and
    /// includes `stop_byte`.
    ///
    /// ## Errors
    /// - Returns [`StreamReadError::ReadBufferOverflow`] if the allocator does not
    ///   have enough capacity for the collected bytes.
    /// - Returns [`StreamReadError::SocketReadError`] if reading from the stream fails.
    fn seek_until_byte<'buf, 'allocator>(
        &mut self,
        stop_byte: u8,
        allocator: &'allocator mut PrefixArena<'buf>,
    ) -> impl core::future::Future<Output = Result<&'buf mut [u8], StreamReadError<Self::Error>>>
    where
        'allocator: 'buf,
    {
        async move {
            self.seek_until(allocator, |chunk| {
                chunk.iter().position(|&b| b == stop_byte).map(|pos| pos + 1)
            })
            .await
        }
    }

    /// Skips exactly `size` bytes from the stream without storing them.
    ///
    /// ## Errors
    /// - Returns [`StreamReadError::SocketReadError`] if reading from the stream fails
    ///   before `size` bytes are consumed.
    fn skip(&mut self, size: usize) -> impl core::future::Future<Output = Result<(), StreamReadError<Self::Error>>> {
        async move {
            let mut bytes_to_consume = size;

            while bytes_to_consume > 0 {
                let bytes_read = self
                    .read_with(|data: &mut [u8]| {
                        let to_read = core::cmp::min(bytes_to_consume, data.len());
                        (to_read, to_read)
                    })
                    .await
                    .map_err(StreamReadError::SocketReadError)?;
                bytes_to_consume -= bytes_read;
            }

            Ok(())
        }
    }

    /// Skips bytes from the stream until `stop_condition` matches.
    ///
    /// `stop_condition` receives each chunk and may return `Some(len)` to stop after
    /// consuming the first `len` bytes of the current chunk. Returning `None`
    /// consumes the whole chunk and continues reading.
    ///
    /// The returned count includes the bytes consumed from the chunk that satisfied the
    /// stop condition.
    ///
    /// ## Errors
    /// - Returns [`StreamReadError::SocketReadError`] if reading from the stream fails.
    fn skip_until<StopF>(
        &mut self,
        mut stop_condition: StopF,
    ) -> impl core::future::Future<Output = Result<usize, StreamReadError<Self::Error>>>
    where
        StopF: FnMut(&[u8]) -> Option<usize>,
    {
        async move {
            let mut total_consumed = 0;

            while self
                .read_with(|data: &mut [u8]| {
                    if let Some(pos) = stop_condition(data) {
                        // The stop point has been found; stop reading further
                        total_consumed += pos;
                        return (pos, false);
                    }
                    total_consumed += data.len();
                    (data.len(), true)
                })
                .await
                .map_err(StreamReadError::SocketReadError)?
            {}

            Ok(total_consumed)
        }
    }

    /// Skips bytes from the stream until `stop_byte` is found.
    ///
    /// The returned count includes `stop_byte`.
    ///
    /// ## Errors
    /// - Returns [`StreamReadError::SocketReadError`] if reading from the stream fails.
    fn skip_until_byte(
        &mut self,
        stop_byte: u8,
    ) -> impl core::future::Future<Output = Result<usize, StreamReadError<Self::Error>>> {
        async move {
            self.skip_until(|chunk| chunk.iter().position(|&b| b == stop_byte).map(|pos| pos + 1))
                .await
        }
    }

    /// Skips bytes from the stream until `stop_sequence` is found.
    ///
    /// The returned count includes `stop_sequence`.
    ///
    /// ## Errors
    /// - Returns [`StreamReadError::SocketReadError`] if reading from the stream fails.
    fn skip_until_sequence(
        &mut self,
        stop_sequence: &[u8],
    ) -> impl core::future::Future<Output = Result<usize, StreamReadError<Self::Error>>> {
        async move {
            let mut sequence_finder = FindSequence::new(stop_sequence);
            self.skip_until(|chunk| sequence_finder.check_next_slice(chunk)).await
        }
    }
}

impl<T: SocketReadWith + ?Sized> StreamSearch for T {}

/// Re-export the mock stream for testing purposes.
#[cfg(all(test, not(feature = "embassy_impl")))]
pub mod tests {
    use super::*;
    use crate::abstarct_socket::mocks::mock_read_stream::*;
    use embedded_io_async::Read;
    use prefix_arena::PrefixArena;

    #[tokio::test]
    async fn test_seek_until_sequence() {
        const STOP: &[u8] = b"\r\n";
        let mut request_data = b"Hello, World!\r\nThis is a test.\r\n".to_vec();
        let mut stream = MockReadStream::new(&mut request_data);
        let mut buffer = [0u8; 64];
        let mut allocator = PrefixArena::new(&mut buffer);

        let bytes_read = stream
            .seek_until_sequence(STOP, &mut allocator)
            .await
            .expect("Expect no error");

        assert_eq!(bytes_read.len(), b"Hello, World!".len() + STOP.len());
        assert_eq!(bytes_read, b"Hello, World!\r\n");
    }

    #[tokio::test]
    async fn test_read_stop_sequence_only() {
        const STOP: &[u8] = b"\r\n";
        let mut request_data = b"\r\n".to_vec();
        let mut stream = MockReadStream::new(&mut request_data);
        let mut buffer = [0u8; 64];
        let mut allocator = PrefixArena::new(&mut buffer);
        let bytes_read = stream
            .seek_until_sequence(STOP, &mut allocator)
            .await
            .expect("Expect no error");

        assert_eq!(bytes_read.len(), STOP.len());
        assert_eq!(bytes_read, STOP);
    }

    #[tokio::test]
    async fn test_read_eof_when_no_stop_found() {
        const STOP: &[u8] = b"\r\n";
        let mut request_data = b"Hello, World!".to_vec();
        let mut stream = MockReadStream::new(&mut request_data);
        let mut buffer = [0u8; 64];
        let mut allocator = PrefixArena::new(&mut buffer);
        let error = stream
            .seek_until_sequence(STOP, &mut allocator)
            .await
            .expect_err("Expect read error, due to read stream EOF");

        assert!(matches!(error, StreamReadError::SocketReadError(_)));
    }

    #[tokio::test]
    async fn test_read_buffer_overflow() {
        const STOP: &[u8] = b"\r\n";
        let mut request_data = b"Hello, World!\r\n".to_vec();
        let mut stream = MockReadStream::new(&mut request_data);
        let mut buffer = [0u8; 4];
        let mut allocator = PrefixArena::new(&mut buffer);

        let error = stream
            .seek_until_sequence(STOP, &mut allocator)
            .await
            .expect_err("Expect buffer overflow error");

        assert!(matches!(error, StreamReadError::ReadBufferOverflow));
    }

    #[tokio::test]
    async fn test_consume_bytes() {
        let mut request_data = b"Hello, World!\r\n".to_vec();
        let mut stream = MockReadStream::new(&mut request_data);
        let mut buffer = [0u8; 64];

        stream.skip(7).await.expect("Expect no error");

        let read_bytes = stream.read(&mut buffer).await.expect("Expect no error");
        assert_eq!(&buffer[..read_bytes], b"World!\r\n");
    }

    #[tokio::test]
    async fn test_consume_stop() {
        const STOP: u8 = b',';
        let mut request_data = b"Hello, World!\r\n".to_vec();
        let mut stream = MockReadStream::new(&mut request_data);
        let mut buffer = [0u8; 64];

        let consumed = stream
            .skip_until(|chunk| chunk.iter().position(|&b| b == STOP).map(|pos| pos + 1))
            .await
            .expect("Expect no error");
        assert_eq!(consumed, b"Hello,".len());

        let read_bytes = stream.read(&mut buffer).await.expect("Expect no error");
        assert_eq!(&buffer[..read_bytes], b" World!\r\n");
    }

    #[tokio::test]
    async fn test_skip_until_sequence() {
        let mut request_data = b"Hello, World!\r\n".to_vec();
        let mut stream = MockReadStream::new(&mut request_data);
        let mut buffer = [0u8; 64];

        let consumed = stream.skip_until_sequence(b", Wo").await.expect("Expect no error");
        assert_eq!(consumed, b"Hello, Wo".len());

        let read_bytes = stream.read(&mut buffer).await.expect("Expect no error");
        assert_eq!(&buffer[..read_bytes], b"rld!\r\n");
    }

    #[tokio::test]
    async fn test_consume_all_data_if_no_sequence_found() {
        let mut request_data = b"Hello, World!\r\n".to_vec();
        let mut stream = MockReadStream::new(&mut request_data);
        let mut buffer = [0u8; 64];

        let error = stream
            .skip_until_sequence(b"There is no such sequence")
            .await
            .expect_err("Expect error");

        assert!(matches!(error, StreamReadError::SocketReadError(_)));

        let bytes_read = stream.read(&mut buffer).await.expect("Expect Ok(0) due to EOF");
        assert_eq!(bytes_read, 0);
    }
}
