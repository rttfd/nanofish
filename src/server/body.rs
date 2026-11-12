//! The body of a request, read from the connection as the handler asks for it.
//!
//! The server reads only the request head up front. What follows is delimited
//! by `Content-Length` or by chunked transfer coding and is read on demand,
//! straight out of the connection buffer: payload is handed out as slices of
//! that buffer, with no copy and without chunk framing.
//!
//! [`BodyReader`] holds the state of that read and borrows no stream, so it
//! can sit next to a [`ResponseWriter`] that owns the stream. [`RequestBody`]
//! pairs it with a stream for handlers that only read.

use core::ops::Range;
use embedded_io_async::{Read, Write};
use nanofish::{
    codec::{ChunkDecoder, Framing},
    error::Error,
    server::stream::ResponseWriter,
    status_code::StatusCode,
};

/// The interim response that tells a client to go ahead and send its body.
const CONTINUE: &[u8] = b"HTTP/1.1 100 Continue\r\n\r\n";

/// How much of an unread body the server reads and discards after answering,
/// so that closing the connection does not reset it under the response.
pub(super) const DRAIN_LIMIT_BYTES: usize = 16 * 1024;

/// Why the connection failed while the body was read, and so how the peer is
/// answered if the handler returns the error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BodyFailure {
    /// The peer is still there and gets this status.
    Status(StatusCode),
    /// The peer is gone or the stream failed: nothing can be sent.
    PeerGone,
}

/// The status to answer a handler's failure with, if the peer can be answered.
///
/// An error that comes from a failed body read (`failure` says so) is
/// answered like the failure: `400` for bad framing, `413` for a body that
/// does not fit, nothing for a peer that is gone. Any other error is the
/// handler's own, a `500`.
pub(super) const fn reply_status(
    failure: Option<BodyFailure>,
    error: &Error,
) -> Option<StatusCode> {
    let from_body = matches!(
        error,
        Error::BufferOverflow | Error::InvalidRequest(_) | Error::TcpError(_)
    );
    match failure {
        Some(BodyFailure::PeerGone) if from_body => None,
        Some(BodyFailure::Status(status)) if from_body => Some(status),
        _ => Some(StatusCode::InternalServerError),
    }
}

/// How much of the body is left.
#[derive(Debug, Clone, Copy)]
enum Mode {
    /// The whole body has been read.
    Done,
    /// This many bytes of a `Content-Length` body.
    Length(usize),
    /// A chunked body.
    Chunked(ChunkDecoder),
}

/// The buffer a body is read into, and how it is framed.
pub struct BodyParts<'b> {
    /// The part of the connection buffer behind the request head.
    pub(super) buf: &'b mut [u8],
    /// How many bytes at the start of `buf` arrived together with the head.
    pub(super) buffered: usize,
    pub(super) framing: Framing,
    /// The client waits for `100 Continue` before it sends the body.
    pub(super) expect_continue: bool,
}

impl BodyParts<'_> {
    /// A body that is empty, read into `buf`.
    #[cfg(test)]
    pub(crate) const fn empty(buf: &mut [u8]) -> BodyParts<'_> {
        BodyParts {
            buf,
            buffered: 0,
            framing: Framing::None,
            expect_continue: false,
        }
    }
}

/// The state of reading one request body, without the stream.
///
/// A [`StreamingHandler`](nanofish::server::StreamingHandler) gets one next to
/// its [`ResponseWriter`], which owns the stream: read with
/// [`chunk`](Self::chunk) or [`all`](Self::all), passing the writer.
///
/// Chunked and `Content-Length` bodies read the same way; chunk framing and
/// trailers never reach the handler. Payload is borrowed from the connection
/// buffer, so a body of any size streams through a buffer of any size.
///
/// A request that announces `Expect: 100-continue` gets its interim response
/// when the body is first read, so a handler that rejects the request without
/// reading never asks the client to send it.
pub struct BodyReader<'a> {
    buf: &'a mut [u8],
    /// Start of the bytes read from the stream but not yet processed.
    pos: usize,
    /// End of those bytes.
    end: usize,
    /// Bytes at the start of `buf` that hold collected payload and must not
    /// be overwritten.
    keep: usize,
    mode: Mode,
    expect_continue: bool,
    /// Payload has been handed out by [`chunk`](Self::chunk).
    delivered: bool,
    /// [`all`](Self::all) has collected the body, which is `buf[..n]`.
    collected: Option<usize>,
    failure: Option<BodyFailure>,
}

impl<'a> BodyReader<'a> {
    pub(super) const fn new(
        BodyParts {
            buf,
            buffered,
            framing,
            expect_continue,
        }: BodyParts<'a>,
    ) -> Self {
        let mode = match framing {
            Framing::None | Framing::Length(0) => Mode::Done,
            Framing::Length(length) => Mode::Length(length),
            Framing::Chunked => Mode::Chunked(ChunkDecoder::new()),
        };
        Self {
            buf,
            pos: 0,
            end: buffered,
            keep: 0,
            mode,
            expect_continue,
            delivered: false,
            collected: None,
            failure: None,
        }
    }

    /// Whether the whole body has been read.
    #[must_use]
    pub const fn is_done(&self) -> bool {
        matches!(self.mode, Mode::Done)
    }

    /// Whether bytes of another request followed this one.
    pub(super) const fn pipelined(&self) -> bool {
        self.is_done() && self.pos < self.end
    }

    /// How the peer is answered if the handler fails with the error of a
    /// read, if the read said so.
    pub(super) const fn failure(&self) -> Option<BodyFailure> {
        self.failure
    }

    const fn fail(&mut self, failure: BodyFailure, error: Error) -> Error {
        self.failure = Some(failure);
        error
    }

    /// The error for a body that does not fit the buffer.
    const fn overflow(&mut self) -> Error {
        self.fail(
            BodyFailure::Status(StatusCode::RequestEntityTooLarge),
            Error::BufferOverflow,
        )
    }

    /// The next piece of the body, read from `response`'s connection, or
    /// `None` once all of it has been read.
    ///
    /// The piece is decoded payload, never empty, borrowed from the
    /// connection buffer: its size depends on how the data arrived, not on
    /// the chunking.
    ///
    /// # Errors
    ///
    /// - [`Error::InvalidRequest`] if the chunk framing is malformed (the
    ///   server answers `400`) or the peer closes the connection before the
    ///   body is complete.
    /// - [`Error::TcpError`] if the stream fails.
    /// - [`Error::InvalidState`] after [`all`](Self::all).
    /// - [`Error::BufferOverflow`] if the buffer is too small to hold even one
    ///   piece of framing.
    pub async fn chunk<'b, S, const HEAD_SIZE: usize>(
        &'b mut self,
        response: &mut ResponseWriter<'_, S, HEAD_SIZE>,
    ) -> Result<Option<&'b [u8]>, Error>
    where
        S: Read + Write,
    {
        self.chunk_from(response.stream_mut()).await
    }

    /// The whole body as one slice, read from `response`'s connection and
    /// decoded in place in the connection buffer. Calling it again returns
    /// the same slice.
    ///
    /// A body that does not fit the buffer is an error that the server
    /// answers with `413` when the handler returns it. A `Content-Length`
    /// that cannot fit fails at once, without reading.
    ///
    /// # Errors
    ///
    /// - [`Error::BufferOverflow`] if the body does not fit.
    /// - [`Error::InvalidState`] if [`chunk`](Self::chunk) already returned
    ///   part of the body.
    /// - The errors of [`chunk`](Self::chunk).
    pub async fn all<'b, S, const HEAD_SIZE: usize>(
        &'b mut self,
        response: &mut ResponseWriter<'_, S, HEAD_SIZE>,
    ) -> Result<&'b [u8], Error>
    where
        S: Read + Write,
    {
        self.all_from(response.stream_mut()).await
    }

    pub(super) async fn chunk_from<'b, S>(
        &'b mut self,
        stream: &mut S,
    ) -> Result<Option<&'b [u8]>, Error>
    where
        S: Read + Write,
    {
        if self.collected.is_some() {
            return Err(Error::InvalidState("Request body already collected"));
        }
        match self.next_range(stream).await? {
            Some(range) => {
                self.delivered = true;
                Ok(Some(&self.buf[range]))
            }
            None => Ok(None),
        }
    }

    pub(super) async fn all_from<'b, S>(&'b mut self, stream: &mut S) -> Result<&'b [u8], Error>
    where
        S: Read + Write,
    {
        if let Some(len) = self.collected {
            return Ok(&self.buf[..len]);
        }
        if self.delivered {
            return Err(Error::InvalidState("Request body already partly read"));
        }
        if let Mode::Length(length) = self.mode
            && length > self.buf.len()
        {
            return Err(self.overflow());
        }

        // Payload is moved down behind what was collected before it. The
        // decoder never runs ahead of the input it has consumed, so the
        // destination never overtakes the source.
        let mut len = 0;
        while let Some(range) = self.next_range(stream).await? {
            // Collected payload is always behind the bytes already consumed.
            assert!(len <= range.start);
            let size = range.len();
            if range.start != len {
                self.buf.copy_within(range, len);
            }
            len += size;
            self.keep = len;
        }
        assert!(len <= self.pos);
        assert!(len <= self.buf.len());
        self.collected = Some(len);
        Ok(&self.buf[..len])
    }

    /// Read and discard what is left of the body, up to `limit` bytes.
    ///
    /// Does nothing for a client still waiting for `100 Continue`: it will
    /// not send the body, so waiting for it would only stall.
    ///
    /// # Errors
    ///
    /// The errors of [`chunk`](Self::chunk), and [`Error::BufferOverflow`]
    /// when more than `limit` bytes remain.
    pub(super) async fn drain<S>(&mut self, stream: &mut S, limit: usize) -> Result<(), Error>
    where
        S: Read + Write,
    {
        if self.expect_continue {
            return Ok(());
        }
        let mut skipped = 0usize;
        while let Some(range) = self.next_range(stream).await? {
            skipped = skipped.saturating_add(range.len());
            if skipped > limit {
                return Err(Error::BufferOverflow);
            }
        }
        Ok(())
    }

    /// The range of `buf` holding the next payload, or `None` at the end of
    /// the body.
    async fn next_range<S>(&mut self, stream: &mut S) -> Result<Option<Range<usize>>, Error>
    where
        S: Read + Write,
    {
        loop {
            match self.mode {
                Mode::Done => return Ok(None),
                Mode::Length(0) => {
                    self.mode = Mode::Done;
                    return Ok(None);
                }
                Mode::Length(remaining) => {
                    if self.pos == self.end {
                        self.fill(stream).await?;
                    }
                    assert!(self.pos < self.end);
                    let take = remaining.min(self.end - self.pos);
                    let start = self.pos;
                    self.pos += take;
                    self.mode = Mode::Length(remaining - take);
                    return Ok(Some(start..start + take));
                }
                Mode::Chunked(mut decoder) => {
                    if self.pos == self.end {
                        self.fill(stream).await?;
                    }
                    let step = match decoder.step(&self.buf[self.pos..self.end]) {
                        Ok(step) => step,
                        Err(reason) => {
                            return Err(self.fail(
                                BodyFailure::Status(StatusCode::BadRequest),
                                Error::InvalidRequest(reason),
                            ));
                        }
                    };
                    let base = self.pos;
                    self.pos += step.consumed;
                    assert!(self.pos <= self.end);
                    self.mode = if decoder.is_done() {
                        Mode::Done
                    } else {
                        Mode::Chunked(decoder)
                    };
                    match step.data {
                        Some(range) if !range.is_empty() => {
                            return Ok(Some(base + range.start..base + range.end));
                        }
                        _ if decoder.is_done() => return Ok(None),
                        _ => {}
                    }
                }
            }
        }
    }

    /// Read more of the body from the stream into the free part of the
    /// buffer, first moving unprocessed bytes (a framing line cut in two)
    /// down against the collected payload.
    async fn fill<S>(&mut self, stream: &mut S) -> Result<(), Error>
    where
        S: Read + Write,
    {
        assert!(self.keep <= self.pos);
        assert!(self.pos <= self.end);
        assert!(self.end <= self.buf.len());
        if self.pos > self.keep {
            self.buf.copy_within(self.pos..self.end, self.keep);
            self.end = self.keep + (self.end - self.pos);
            self.pos = self.keep;
        }
        if self.end == self.buf.len() {
            return Err(self.overflow());
        }

        if self.expect_continue {
            self.expect_continue = false;
            let sent = match stream.write_all(CONTINUE).await {
                Ok(()) => stream.flush().await,
                Err(error) => Err(error),
            };
            if let Err(error) = sent {
                return Err(self.fail(BodyFailure::PeerGone, Error::io(error)));
            }
        }

        match stream.read(&mut self.buf[self.end..]).await {
            Ok(0) => Err(self.fail(
                BodyFailure::PeerGone,
                Error::InvalidRequest("Incomplete request"),
            )),
            Ok(read) => {
                self.end += read;
                assert!(self.end <= self.buf.len());
                Ok(())
            }
            Err(error) => Err(self.fail(BodyFailure::PeerGone, Error::io(error))),
        }
    }
}

/// The body of a request, with the connection to read it from.
///
/// A handler gets one with every request. It may ignore it (the server
/// discards what is left after the response), take the body as it comes with
/// [`chunk`](Self::chunk), or collect all of it with [`all`](Self::all).
/// See [`BodyReader`] for how the body is read.
pub struct RequestBody<'a, S> {
    stream: &'a mut S,
    reader: BodyReader<'a>,
}

impl<'a, S> RequestBody<'a, S> {
    pub(crate) const fn new(stream: &'a mut S, parts: BodyParts<'a>) -> Self {
        Self {
            stream,
            reader: BodyReader::new(parts),
        }
    }

    /// The connection and the state of the read, to answer and clean up after
    /// the handler.
    pub(super) const fn into_parts(self) -> (&'a mut S, BodyReader<'a>) {
        (self.stream, self.reader)
    }

    /// Whether the whole body has been read.
    #[must_use]
    pub const fn is_done(&self) -> bool {
        self.reader.is_done()
    }
}

impl<S> RequestBody<'_, S>
where
    S: Read + Write,
{
    /// The next piece of the body, or `None` once all of it has been read.
    ///
    /// The piece is decoded payload, never empty, borrowed from the
    /// connection buffer.
    ///
    /// # Errors
    ///
    /// - [`Error::InvalidRequest`] if the chunk framing is malformed (the
    ///   server answers `400`) or the peer closes the connection before the
    ///   body is complete.
    /// - [`Error::TcpError`] if the stream fails.
    /// - [`Error::InvalidState`] after [`all`](Self::all).
    /// - [`Error::BufferOverflow`] if the buffer is too small to hold even one
    ///   piece of framing.
    pub async fn chunk(&mut self) -> Result<Option<&[u8]>, Error> {
        self.reader.chunk_from(self.stream).await
    }

    /// The whole body as one slice, decoded in place in the connection
    /// buffer. Calling it again returns the same slice.
    ///
    /// A body that does not fit the buffer is an error that the server
    /// answers with `413` when the handler returns it. A `Content-Length`
    /// that cannot fit fails at once, without reading.
    ///
    /// # Errors
    ///
    /// - [`Error::BufferOverflow`] if the body does not fit.
    /// - [`Error::InvalidState`] if [`chunk`](Self::chunk) already returned
    ///   part of the body.
    /// - The errors of [`chunk`](Self::chunk).
    pub async fn all(&mut self) -> Result<&[u8], Error> {
        self.reader.all_from(self.stream).await
    }
}
