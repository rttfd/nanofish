use crate::socket::{SocketWaitReadReady, SocketWaitWriteReady};
use crate::web_socket::header::*;
use crate::web_socket::header_reader::*;
use core::fmt::Debug;
use defmt_or_log as log;
use embedded_io_async::{ErrorType, Read, ReadExactError, ReadReady, Write, WriteReady};

#[derive(Debug, PartialEq, Clone, Copy)]
enum PipeState {
    Open,
    Closed,
}

/// Errors that can occur during WebSocket protocol parsing
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum WebSocketState {
    /// The WebSocket connection is open and both sending and receiving pipes are operational.
    Open,
    /// The WebSocket connection is in the process of closing, where the local side has initiated
    /// the close handshake but is still waiting for the remote side to acknowledge it.
    Closing,
    /// The WebSocket connection has been closed by the remote side, where the local side has
    /// received a close frame from the remote side but has not yet sent its own close frame in
    /// response.
    ClosedByRemoteSide,
    /// The WebSocket connection is fully closed, where both sending and receiving pipes are closed,
    /// either due to a successful close handshake or due to an unrecoverable error that caused
    /// both sides to close the connection.
    Closed,
}

/// Errors that can occur during WebSocket operations
pub enum WebSocketError<E> {
    /// The WebSocket frame header is invalid, which can occur if the header does not conform to the
    /// WebSocket protocol specifications, such as having an invalid opcode, incorrect payload length
    /// encoding, or missing required fields.
    MalformedHeader,
    /// The WebSocket buffer overflowed, which can occur if the payload length of a WebSocket frame
    /// exceeds the maximum allowed size or if there is an attempt to read more data than the buffer
    /// can hold.
    BufferOverflow,
    /// The WebSocket connection has been closed, which can occur if the close handshake has been
    /// completed or if an unrecoverable error has occurred that caused the connection to be
    /// closed.
    Closed,
    /// An error occurred while performing socket operations, such as reading from or writing to
    /// the underlying socket. The specific error is encapsulated in the `E` type, which can
    /// represent various socket
    SocketError(E),
}

#[cfg(feature = "defmt")]
impl<E: Debug> defmt::Format for WebSocketError<E> {
    fn format(&self, f: defmt::Formatter<'_>) {
        match self {
            WebSocketError::MalformedHeader => defmt::write!(f, "Invalid WebSocket header"),
            WebSocketError::BufferOverflow => defmt::write!(f, "WebSocket buffer overflow"),
            WebSocketError::Closed => defmt::write!(f, "WebSocket closed"),
            WebSocketError::SocketError(e) => {
                defmt::write!(f, "WebSocket socket error: {:?}", defmt::Debug2Format(e))
            }
        }
    }
}

impl<E: Debug> Debug for WebSocketError<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            WebSocketError::MalformedHeader => write!(f, "Invalid WebSocket header"),
            WebSocketError::BufferOverflow => write!(f, "WebSocket buffer overflow"),
            WebSocketError::Closed => write!(f, "WebSocket closed"),
            WebSocketError::SocketError(e) => write!(f, "WebSocket socket error: {:?}", e),
        }
    }
}

impl<E: Debug> core::fmt::Display for WebSocketError<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            WebSocketError::MalformedHeader => write!(f, "Malformed WebSocket header"),
            WebSocketError::BufferOverflow => write!(f, "WebSocket buffer overflow"),
            WebSocketError::Closed => write!(f, "WebSocket closed"),
            WebSocketError::SocketError(e) => write!(f, "WebSocket socket error: {:?}", e),
        }
    }
}

impl<E: embedded_io_async::Error> embedded_io_async::Error for WebSocketError<E> {
    fn kind(&self) -> embedded_io_async::ErrorKind {
        match self {
            WebSocketError::MalformedHeader => embedded_io_async::ErrorKind::InvalidData,
            WebSocketError::BufferOverflow => embedded_io_async::ErrorKind::OutOfMemory,
            WebSocketError::Closed => embedded_io_async::ErrorKind::BrokenPipe,
            WebSocketError::SocketError(e) => e.kind(),
        }
    }
}

impl<E: Debug> From<WebSocketProtoError> for WebSocketError<E> {
    fn from(_err: WebSocketProtoError) -> Self {
        WebSocketError::MalformedHeader
    }
}

impl<E: embedded_io_async::Error> From<E> for WebSocketError<E> {
    fn from(err: E) -> Self {
        WebSocketError::SocketError(err)
    }
}

impl<E: embedded_io_async::Error> From<ReadExactError<E>> for WebSocketError<E> {
    fn from(err: ReadExactError<E>) -> Self {
        match err {
            ReadExactError::UnexpectedEof => WebSocketError::Closed,
            ReadExactError::Other(e) => WebSocketError::SocketError(e),
        }
    }
}

/// A struct representing a WebSocket connection, which wraps an underlying socket and manages
/// the WebSocket protocol state, including frame parsing, payload reading, and close handshake
/// handling.
pub struct WebSocket<'s, S> {
    socket: &'s mut S,
    receiving_state: PipeState,
    sending_state: PipeState,
    recv_header_buffer: [u8; MAX_WS_FRAME_HEADER_SIZE],
    send_header_buffer: [u8; MAX_WS_FRAME_HEADER_SIZE],
    active_payload_reader: Option<WSPayloadReader>,
}

impl<'s, S> WebSocket<'s, S>
where
    S: ErrorType,
{
    /// Creates a new WebSocket instance wrapping the provided socket. The WebSocket is initialized
    /// with both sending and receiving pipes in the open state, and with empty header buffers and
    /// no active payload reader.
    /// ## Parameters
    /// - `socket`: A mutable reference to the underlying socket that the WebSocket will wrap and manage.
    /// ## Returns
    /// - A new instance of `WebSocket` that is ready to perform WebSocket communication using the provided socket.
    /// ## Notes
    /// - The provided socket must implement the necessary traits for reading and writing data, as
    ///   well as error handling, for the WebSocket to function correctly. The WebSocket will manage the
    ///   protocol state and handle WebSocket-specific operations, while delegating the actual data transmission
    ///   to the underlying socket.
    pub const fn new(socket: &'s mut S) -> Self {
        Self {
            socket,
            receiving_state: PipeState::Open,
            sending_state: PipeState::Open,
            recv_header_buffer: [0; MAX_WS_FRAME_HEADER_SIZE],
            send_header_buffer: [0; MAX_WS_FRAME_HEADER_SIZE],
            active_payload_reader: None,
        }
    }

    /// Performs close handshake and releases the underlying socket
    pub async fn release(mut self) -> (&'s mut S, Result<(), WebSocketError<S::Error>>)
    where
        S: Write + Read + ReadReady,
        WebSocketError<S::Error>: From<ReadExactError<S::Error>>,
    {
        let res = self.close().await;
        (self.socket, res)
    }

    fn close_on_critical_error<E: Debug>(&mut self, e: E) -> E
    where
        S::Error: Debug,
    {
        log::error!(
            "WebSocket: Close due to unrecoverable error occurred: {:?}",
            log::Debug2Format(&e)
        );
        self.receiving_state = PipeState::Closed;
        self.sending_state = PipeState::Closed;
        e
    }

    /// Closes the WebSocket connection gracefully by performing the close handshake process. This involves
    /// sending a close frame to the remote side, flushing any remaining data in the read stream, and waiting
    /// for a close frame from the remote side if necessary. The method ensures that all pending data is sent
    /// and acknowledged before closing the connection, and it handles various states of the WebSocket
    /// connection to ensure a proper close handshake is performed.
    /// ## Returns
    /// - `Ok(())` if the WebSocket connection was closed successfully.
    /// - `WebSocketError::Closed` if the WebSocket connection is already closed.
    /// - `WebSocketError::InvalidHeader` if an invalid WebSocket frame header is encountered during
    ///   the close handshake process.
    /// - `WebSocketError::SocketError` if an error occurs while performing socket operations during
    ///   the close handshake process.
    /// ## Notes
    /// - The close handshake process involves multiple steps, including sending a close frame, flushing
    ///   the read stream, and waiting for a close frame from the remote side. The method handles different
    ///   states of the WebSocket connection to ensure that the close handshake is performed correctly, and
    ///   it uses the underlying socket for data transmission while managing the WebSocket protocol state
    ///   internally.
    pub async fn close(&mut self) -> Result<(), WebSocketError<S::Error>>
    where
        S: Write + Read + ReadReady,
        S::Error: Debug,
        WebSocketError<S::Error>: From<ReadExactError<S::Error>>,
    {
        if self.receiving_state == PipeState::Open && self.sending_state == PipeState::Open {
            // Flush any remaining data in the read stream. This allow to make sure we read any pending
            //close frame.
            self.flush_read_stream().await?;

            // We are the first to initiate close
            self.send_close_frame().await?;
            self.sending_state = PipeState::Closed;

            self.wait_for_close_frame().await?;
            self.receiving_state = PipeState::Closed;
            return Ok(());
        } else if self.receiving_state == PipeState::Open {
            // Somehow the close procedure is not finished yet and we are waitng for the close frame
            self.wait_for_close_frame().await?;
            self.receiving_state = PipeState::Closed;
            return Ok(());
        } else if self.sending_state == PipeState::Open {
            // We need to send the close frame as we have received the close frame already
            self.send_close_frame().await?;
            self.sending_state = PipeState::Closed;
        }

        Ok(())
    }

    /// Retrieves the current state of the WebSocket connection based on the states of the sending and
    /// receiving pipes.
    pub fn state(&self) -> WebSocketState {
        match (self.sending_state, self.receiving_state) {
            (PipeState::Open, PipeState::Open) => WebSocketState::Open,
            (PipeState::Closed, PipeState::Open) => WebSocketState::Closing,
            (PipeState::Open, PipeState::Closed) => WebSocketState::ClosedByRemoteSide,
            (PipeState::Closed, PipeState::Closed) => WebSocketState::Closed,
        }
    }

    async fn read_header(&mut self) -> Result<WSFrameHeader, WebSocketError<S::Error>>
    where
        S: Read,
        WebSocketError<S::Error>: From<ReadExactError<S::Error>>,
    {
        let mut read_pos: usize = 0;
        let mut header_size: usize = MIN_WS_FRAME_HEADER_SIZE;

        loop {
            self.socket
                .read_exact(&mut self.recv_header_buffer[read_pos..header_size])
                .await
                .map_err(|e| self.close_on_critical_error(e))?;

            match read_frame_header(&self.recv_header_buffer[..header_size]) {
                Ok((header, _)) => {
                    return Ok(header);
                }
                Err(WebSocketProtoError::NotEnoughData(expected_size)) => {
                    // Next iteration will read more data
                    read_pos = header_size;
                    header_size = expected_size;
                    log::assert!(read_pos < header_size);
                    continue;
                }
                Err(_) => {
                    return {
                        self.close_on_critical_error(());
                        Err(WebSocketError::MalformedHeader)
                    };
                }
            };
        }
    }

    async fn send_close_frame(&mut self) -> Result<(), WebSocketError<S::Error>>
    where
        S: Write,
    {
        log::debug!("WebSocket: Sending close frame");
        let header_size = write_frame_header(0, &mut self.send_header_buffer, WSOpcode::Close, 1);

        self.socket
            .write_all(&self.send_header_buffer[..header_size])
            .await
            .map_err(|e| self.close_on_critical_error(e))?;

        Ok(())
    }

    async fn flush_read_stream(&mut self) -> Result<(), WebSocketError<S::Error>>
    where
        S: Read + ReadReady,
        WebSocketError<S::Error>: From<ReadExactError<S::Error>>,
    {
        // Read out previously active payload reader if any
        if let Some(mut payload_reader) = self.active_payload_reader.take() {
            log::warn!(
                "WebSocket: Flushing incomplete payload reader with {} bytes remaining",
                payload_reader.payload_len()
            );
            // Reuse the existing recv_header_buffer to read data into
            let mut buf = self.recv_header_buffer;

            while !payload_reader.is_complete() {
                let read_len: usize = payload_reader.payload_len();
                let actual_read_len = core::cmp::min(read_len, buf.len());
                self.socket
                    .read_exact(&mut buf[..actual_read_len])
                    .await
                    .map_err(|e| self.close_on_critical_error(e))?;
                // We don't need the decoded payload, just consume it
                payload_reader.consume_payload(actual_read_len);
            }
        }

        while self.socket.read_ready().map_err(|e| self.close_on_critical_error(e))? {
            log::trace!("WebSocket: Flushing additional data from read stream");

            // There is more data to read, continue flushing
            let header: WSFrameHeader = self.read_header().await?;
            if header.opcode == WSOpcode::Close {
                log::trace!("WebSocket: Close frame received during flush of read stream");
                self.receiving_state = PipeState::Closed;
            }
            let mut payload_reader = WSPayloadReader::from_header(&header);

            // Read data
            let mut buf = self.recv_header_buffer;
            while !payload_reader.is_complete() {
                let read_len: usize = payload_reader.payload_len();
                let actual_read_len = core::cmp::min(read_len, buf.len());
                self.socket
                    .read_exact(&mut buf[..actual_read_len])
                    .await
                    .map_err(|e| self.close_on_critical_error(e))?;
                // We don't need the decoded payload, just consume it
                payload_reader.consume_payload(actual_read_len);
            }
        }
        Ok(())
    }

    async fn wait_for_close_frame(&mut self) -> Result<(), WebSocketError<S::Error>>
    where
        S: Read,
        WebSocketError<S::Error>: From<ReadExactError<S::Error>>,
    {
        log::debug!("WebSocket: Waiting for close frame from remote side");
        loop {
            let header: WSFrameHeader = self.read_header().await?;
            let mut payload_reader = WSPayloadReader::from_header(&header);

            let mut buf = [0u8; 128];
            while !payload_reader.is_complete() {
                let read_len: usize = payload_reader.payload_len();
                let actual_read_len = core::cmp::min(read_len, buf.len());
                self.socket
                    .read_exact(&mut buf[..actual_read_len])
                    .await
                    .map_err(|e| self.close_on_critical_error(e))?;
                payload_reader.decode_payload_in_place(&mut buf[..actual_read_len]);
            }

            if header.opcode == WSOpcode::Close {
                log::debug!("WebSocket: Close frame received");
                return Ok(());
            }
        }
    }

    /// Retrieves the active payload reader if any, otherwise reads a new frame header
    /// and creates a new payload reader.
    ///
    /// Side effect: If the received header is a close frame, the receiving pipe state is marked as
    /// closed but the reader is still returned.
    ///
    /// ### Errors:
    /// - `WebSocketError::Closed`: If the receiving pipe is closed.
    /// - `WebSocketError::InvalidHeader`: If the frame header is invalid.
    /// - `WebSocketError::SocketError`: If there is an error while reading from the underlying socket
    async fn get_active_payload_reader(&mut self) -> Result<WSPayloadReader, WebSocketError<S::Error>>
    where
        S: Read,
        WebSocketError<S::Error>: From<ReadExactError<S::Error>>,
    {
        if let Some(payload_reader) = self.active_payload_reader.take() {
            log::trace!("WebSocket: Prossede reading next binary frame portion");
            Ok(payload_reader)
        } else {
            if self.receiving_state == PipeState::Closed {
                return Err(WebSocketError::Closed);
            }
            log::trace!("WebSocket: Reading new binary frame");
            let header: WSFrameHeader = self.read_header().await?;

            if header.opcode == WSOpcode::Close {
                // Mark receiving pipe as closed
                log::debug!("WebSocket: The close frame received from remote side");
                self.receiving_state = PipeState::Closed;
            }

            Ok(WSPayloadReader::from_header(&header))
        }
    }

    /// Sets the provided payload reader as an active one in case the provided
    /// one is not complete yet, otherwise doses nothing.
    ///
    /// ### Panics:
    /// - If there is already an active payload reader stored
    #[inline]
    fn set_active_payload_reader(&mut self, payload_reader: WSPayloadReader) {
        if payload_reader.is_complete() {
            // No need to store completed payload reader
            return;
        }

        log::debug_assert!(
            self.active_payload_reader.is_none(),
            "WebSocket: Attempt to overwrite active payload reader stored"
        );
        self.active_payload_reader.replace(payload_reader);
    }
}

impl<'s, S> ErrorType for WebSocket<'s, S>
where
    S: ErrorType,
{
    type Error = WebSocketError<S::Error>;
}

impl<'s, S> Read for WebSocket<'s, S>
where
    S: Read + ErrorType,
    WebSocketError<S::Error>: From<ReadExactError<S::Error>>,
    S::Error: Debug,
{
    /// Reads data from the WebSocket stream to the provided buffer. Reading will stop when either the buffer is full
    /// or the current WebSocket frame is fully read.
    /// Returns the number of bytes read.
    ///
    //// ### Errors:
    /// - `WebSocketError::Closed`: If the WebSocket receiving pipe is closed.
    /// - `WebSocketError::InvalidHeader`: If the frame header is invalid.
    /// - `WebSocketError::SocketError`: If there is an error while reading from the underlying socket.
    ///
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, WebSocketError<S::Error>> {
        log::trace!("WebSocket: Reading binary frame to the buffer of size {}", buf.len());
        let mut payload_reader = self.get_active_payload_reader().await?;

        let read_len: usize = payload_reader.payload_len();
        let actual_read_len = core::cmp::min(read_len, buf.len());
        self.socket
            .read_exact(&mut buf[..actual_read_len])
            .await
            .map_err(|e| self.close_on_critical_error(e))?;

        payload_reader.decode_payload_in_place(&mut buf[0..actual_read_len]);

        self.set_active_payload_reader(payload_reader);
        Ok(actual_read_len)
    }
}

impl<'s, S> Write for WebSocket<'s, S>
where
    S: Write + ErrorType,
    S::Error: Debug,
{
    /// Writes data to the WebSocket stream.
    /// This method sends the data as a binary WebSocket frame.
    /// Returns the number of bytes written.
    ///
    /// ### Error:
    /// - `WebSocketError::Closed`: If the WebSocket sending pipe is closed.
    /// - `WebSocketError::SocketError`: If there is an error while writing to the underlying socket.
    ///
    async fn write(&mut self, buf: &[u8]) -> Result<usize, WebSocketError<S::Error>> {
        if self.sending_state == PipeState::Closed {
            return Err(WebSocketError::Closed);
        }
        log::trace!("WebSocket: Writing binary frame of size {}", buf.len());
        let header_size = write_frame_header(buf.len(), &mut self.send_header_buffer, WSOpcode::Binary, 1);

        self.socket
            .write_all(&self.send_header_buffer[..header_size])
            .await
            .map_err(|e| self.close_on_critical_error(e))?;

        self.socket
            .write_all(buf)
            .await
            .map_err(|e| self.close_on_critical_error(e))?;

        Ok(buf.len())
    }

    #[inline]
    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.socket.flush().await.map_err(|e| self.close_on_critical_error(e))?;
        Ok(())
    }

    #[inline]
    async fn write_all(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
        self.write(buf).await.map_err(|e| self.close_on_critical_error(e))?;
        Ok(())
    }
}

impl<S> WriteReady for WebSocket<'_, S>
where
    S: WriteReady,
    S::Error: Debug,
{
    fn write_ready(&mut self) -> Result<bool, WebSocketError<S::Error>> {
        if self.sending_state == PipeState::Closed {
            return Err(WebSocketError::Closed);
        }
        self.socket
            .write_ready()
            .map_err(|e| WebSocketError::SocketError(self.close_on_critical_error(e)))
    }
}

impl<S> ReadReady for WebSocket<'_, S>
where
    S: ReadReady,
    S::Error: Debug,
{
    fn read_ready(&mut self) -> Result<bool, WebSocketError<S::Error>> {
        if self.active_payload_reader.is_some() {
            // There is an active payload reader, so there still is pending data to read even if socket is closed.
            return Ok(true);
        }

        if self.receiving_state == PipeState::Closed {
            return Err(WebSocketError::Closed);
        }
        // We are ready to read if there is an active payload reader or the underlying socket is ready to read
        self.socket
            .read_ready()
            .map_err(|e| WebSocketError::SocketError(self.close_on_critical_error(e)))
    }
}

impl<S> SocketWaitReadReady for WebSocket<'_, S>
where
    S: SocketWaitReadReady,
{
    async fn wait_read_ready(&mut self) -> Result<(), Self::Error> {
        if self.active_payload_reader.is_some() {
            // There is an active payload reader, so there still is pending data to read even if socket is closed.
            // Return immediately without waiting for the socket to be ready.
            return Ok(());
        }
        if self.receiving_state == PipeState::Closed {
            log::panic!("WebSocket: Attempt to wait for read ready on closed receiving pipe");
        }
        self.socket
            .wait_read_ready()
            .await
            .map_err(|e| WebSocketError::SocketError(self.close_on_critical_error(e)))
    }
}

impl<S> SocketWaitWriteReady for WebSocket<'_, S>
where
    S: SocketWaitWriteReady,
{
    async fn wait_write_ready(&mut self) -> Result<(), Self::Error> {
        if self.sending_state == PipeState::Closed {
            log::panic!("WebSocket: Attempt to wait for write ready on closed sending pipe");
        }
        self.socket
            .wait_write_ready()
            .await
            .map_err(|e| WebSocketError::SocketError(self.close_on_critical_error(e)))
    }
}
