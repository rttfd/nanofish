//! Chunked request bodies: send a body piece by piece from a
//! [`BodySource`], without buffering it and without knowing its length.
//!
//! The mirror of [`BodySink`](nanofish::client::BodySink): a sink receives a
//! response as it is read; a source produces a request as it is written.

use nanofish::error::Error;

/// Produces the body of a chunked upload, one piece at a time.
///
/// The client sends each returned piece as one chunk, so a body of any size
/// passes through in bounded memory. It is driven by
/// [`HttpClient::request_upload`](nanofish::HttpClient::request_upload) and
/// friends; the source decides what a piece is (a flash page, a sensor
/// reading, whatever the caller has at hand).
#[expect(
    async_fn_in_trait,
    reason = "no_std embedded executors are single-threaded, so no `Send` bound is wanted"
)]
pub trait BodySource {
    /// The next piece of the body, or `None` when the body is done.
    ///
    /// An empty slice may be returned; it is skipped. Return an error to
    /// abandon the upload; the caller then drops the connection.
    async fn next_chunk(&mut self) -> Result<Option<&[u8]>, Error>;
}

/// A [`BodySource`] over a slice, yielding it in pieces of at most
/// `max_chunk` bytes.
///
/// Useful for sending a large slice, for example a buffer resident in flash,
/// in bounded writes instead of one `write_all` that a small TCP segment
/// size would split anyway.
#[derive(Debug, Clone, Copy)]
pub struct SliceSource<'a> {
    rest: &'a [u8],
    max_chunk: usize,
}

impl<'a> SliceSource<'a> {
    /// Yield `data` in pieces of at most `max_chunk` bytes. A `max_chunk` of
    /// zero sends the data one byte at a time.
    #[must_use]
    pub const fn new(data: &'a [u8], max_chunk: usize) -> Self {
        Self {
            rest: data,
            max_chunk,
        }
    }
}

impl BodySource for SliceSource<'_> {
    async fn next_chunk(&mut self) -> Result<Option<&[u8]>, Error> {
        if self.rest.is_empty() {
            return Ok(None);
        }
        // At least one byte per piece, so a `max_chunk` of zero still makes
        // progress instead of yielding empty slices forever.
        let take = self.max_chunk.min(self.rest.len()).max(1);
        let (piece, rest) = self.rest.split_at(take);
        self.rest = rest;
        Ok(Some(piece))
    }
}

#[cfg(test)]
mod tests {
    use futures_lite::future::block_on;
    use nanofish::client::BodySource;
    use nanofish::client::upload::SliceSource;

    /// Everything `source` yields.
    fn collect(source: &mut impl BodySource) -> std::vec::Vec<Vec<u8>> {
        let mut pieces = std::vec::Vec::new();
        while let Some(piece) = block_on(source.next_chunk()).unwrap() {
            pieces.push(piece.to_vec());
        }
        pieces
    }

    #[test]
    fn test_slice_source_splits_at_max_chunk() {
        let mut source = SliceSource::new(b"hello world", 5);
        let pieces = collect(&mut source);
        assert_eq!(
            pieces,
            [b"hello".to_vec(), b" worl".to_vec(), b"d".to_vec()]
        );
    }

    #[test]
    fn test_slice_source_trailing_partial_chunk() {
        let mut source = SliceSource::new(b"hello world", 5);
        assert_eq!(block_on(source.next_chunk()).unwrap(), Some(&b"hello"[..]));
        assert_eq!(block_on(source.next_chunk()).unwrap(), Some(&b" worl"[..]));
        // The final piece is shorter than `max_chunk`.
        assert_eq!(block_on(source.next_chunk()).unwrap(), Some(&b"d"[..]));
        assert_eq!(block_on(source.next_chunk()).unwrap(), None);
    }

    #[test]
    fn test_slice_source_empty_input_is_done() {
        let mut source = SliceSource::new(b"", 5);
        assert_eq!(block_on(source.next_chunk()).unwrap(), None);
    }

    #[test]
    fn test_slice_source_zero_max_chunk_still_progresses() {
        let mut source = SliceSource::new(b"ab", 0);
        assert_eq!(block_on(source.next_chunk()).unwrap(), Some(&b"a"[..]));
        assert_eq!(block_on(source.next_chunk()).unwrap(), Some(&b"b"[..]));
        assert_eq!(block_on(source.next_chunk()).unwrap(), None);
    }
}
