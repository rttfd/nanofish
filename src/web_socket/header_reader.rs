#![allow(dead_code)]

use crate::web_socket::header::*;
use core::cmp::min;

pub struct WSHeaderReader {
    header_buf: heapless::Vec<u8, MAX_WS_FRAME_HEADER_SIZE>,
    expected_bytes: usize,
}

pub enum WSHeaderState<E> {
    /// Not enough data to read the complete header. Contains the number of bytes read from provided buffer.
    PendingData(usize),
    /// Successfully read the header. Contains the header and number of bytes read.
    Ready(WSFrameHeader, usize),
    Error(E),
}

impl<E> WSHeaderState<E> {
    pub fn is_ready(&self) -> bool {
        matches!(self, WSHeaderState::Ready(_, _))
    }

    pub fn is_pending(&self) -> bool {
        matches!(self, WSHeaderState::PendingData(_))
    }

    pub fn is_error(&self) -> bool {
        matches!(self, WSHeaderState::Error(_))
    }

    pub fn unwrap_ready(self) -> (WSFrameHeader, usize) {
        match self {
            WSHeaderState::Ready(header, size) => (header, size),
            _ => panic!("Called unwrap_ready on a non-ready state"),
        }
    }

    pub fn unwrap_pending(self) -> usize {
        match self {
            WSHeaderState::PendingData(size) => size,
            _ => panic!("Called unwrap_pending on a non-pending state"),
        }
    }

    pub fn unwrap_error(self) -> E {
        match self {
            WSHeaderState::Error(e) => e,
            _ => panic!("Called unwrap_error on a non-error state"),
        }
    }
}

impl WSHeaderReader {
    /// Creates a new WebSocket frame header reader.
    pub const fn new() -> Self {
        Self {
            header_buf: heapless::Vec::new(),
            expected_bytes: MIN_WS_FRAME_HEADER_SIZE,
        }
    }

    /// Checks if the reader is currently reading a header.
    pub fn is_reading(&self) -> bool {
        !self.header_buf.is_empty()
    }

    /// Tries to read the WebSocket frame header from the provided source buffer.
    ///
    /// Note: This function may be called multiple times as more data becomes available.
    /// It accumulates data internally until the complete header is read.
    /// Returns the state of the read operation.
    /// ## Returns
    /// - `WSHeaderReadState::Ready(WebSocketFrameHeader, usize)`: Successfully read the header.
    ///   Contains the header and number of bytes read during the last call.
    /// - `WSHeaderReadState::PendingData(usize)`: Not enough data to read the complete header.
    ///   Contains the number of bytes read during the last call.
    /// - `WSHeaderReadState::Error(WebSocketProtoError)`: Invalid header or other error.
    /// ## Errors
    /// Returns `WSHeaderReadState::Error(WebSocketProtoError)` if an error occurs while reading the header.
    pub fn try_read_header(&mut self, mut src_buf: &[u8]) -> WSHeaderState<WebSocketProtoError> {
        // Try to reade as many bytes as needed
        let mut read_bytes = 0;
        loop {
            if self.header_buf.is_empty() {
                // Fast path: try to read directly from src_buf if we have enough data.
                // This try will evaluate the minimum number of bytes needed to read the header.
                match read_frame_header(src_buf) {
                    Ok((header, read_size)) => return WSHeaderState::Ready(header, read_size),
                    Err(WebSocketProtoError::NotEnoughData(required_size)) => {
                        // Not enough data, copy what we have and update expected bytes
                        self.header_buf.extend_from_slice(src_buf).unwrap();
                        self.expected_bytes = required_size - src_buf.len();
                        return WSHeaderState::PendingData(src_buf.len());
                    }
                    Err(e) => return WSHeaderState::Error(e),
                }
            } else if src_buf.len() < self.expected_bytes {
                // Stilll not enough data, copy all available data to internal buffer
                self.header_buf.extend_from_slice(src_buf).unwrap();
                // Update expected bytes
                self.expected_bytes -= src_buf.len();
                read_bytes += src_buf.len();
                return WSHeaderState::PendingData(read_bytes);
            } else {
                // We probably have enough data to complete the header
                self.header_buf
                    .extend_from_slice(&src_buf[..self.expected_bytes])
                    .unwrap();

                read_bytes += src_buf.len();
                // Remove consumed data from src_buf
                src_buf = &src_buf[self.expected_bytes..];

                // No more data need to read
                match read_frame_header(self.header_buf.as_slice()) {
                    Ok((header, read_size)) => {
                        // Reset for next read
                        self.header_buf.clear();
                        self.expected_bytes = MIN_WS_FRAME_HEADER_SIZE;
                        // Return the read header
                        return WSHeaderState::Ready(header, read_size);
                    }

                    Err(WebSocketProtoError::NotEnoughData(new_required_size)) => {
                        // Still not enough data, update expected bytes
                        self.expected_bytes = new_required_size - self.header_buf.len();
                        // And make another try
                        continue;
                    }
                    Err(e) => return WSHeaderState::Error(e),
                }
            }
        }
    }
}

pub struct WSPayloadReader {
    masking_key: Option<[u8; WS_MASKING_KEY_LEN]>,
    payload_len: usize,
    read_idx: usize,
}

impl WSPayloadReader {
    pub fn from_header(header: &WSFrameHeader) -> Self {
        Self {
            payload_len: header.payload_len,
            masking_key: header.masking_key,
            read_idx: 0,
        }
    }

    #[inline]
    pub const fn payload_len(&self) -> usize {
        self.payload_len
    }

    pub fn has_masking(&self) -> bool {
        self.masking_key.is_some()
    }

    pub fn masking_key(&self) -> Option<&[u8; 4]> {
        self.masking_key.as_ref()
    }

    pub fn read_bytes_remaining(&self) -> usize {
        self.payload_len - self.read_idx
    }

    pub fn read_bytes(&self) -> usize {
        self.read_idx
    }

    pub fn is_complete(&self) -> bool {
        self.read_idx >= self.payload_len
    }

    pub fn decode_payload_in_place(&mut self, payload_src: &mut [u8]) -> usize {
        let payload_rest = self.payload_len - self.read_idx;
        let size = min(payload_src.len(), payload_rest);

        if let Some(masking_key_bytes) = self.masking_key {
            for (i, payload_byte) in payload_src.iter_mut().enumerate().take(size) {
                let j = (self.read_idx + i) % masking_key_bytes.len();
                let key_byte = masking_key_bytes[j];
                *payload_byte ^= key_byte;
            }
        }

        self.read_idx += size;
        size
    }

    /// Consumes the specified number of bytes from the payload, updating the internal read index.
    ///
    /// ### Returns
    /// - The actual number of bytes consumed, which may be less than the requested size if it exceeds the remaining payload length.
    pub fn consume_payload(&mut self, size: usize) -> usize {
        let payload_rest = self.payload_len - self.read_idx;
        let consume_size = min(size, payload_rest);
        self.read_idx += consume_size;
        consume_size
    }

    /// Consumes all remaining bytes in the payload, updating the internal read index to the end of the payload.
    ///
    /// ### Returns
    /// - The actual number of bytes consumed, which will be equal to the remaining payload length before consumption.
    pub fn consume_all(&mut self) -> usize {
        self.read_idx = self.payload_len;
        self.payload_len
    }
}

// Tests
#[cfg(test)]
mod tests {
    use super::*;
    use crate::web_socket::test_utils::*;

    #[test]
    fn test_heder_reder_create() {
        let reader = WSHeaderReader::new();
        assert_eq!(reader.header_buf.len(), 0);
        assert_eq!(reader.expected_bytes, MIN_WS_FRAME_HEADER_SIZE);
    }

    #[test]
    fn test_header_reader_try_read_header_valid_full() {
        let data = [
            0b10000001, 0xFF, 0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0x12, 0x34, 0x56, 0x78,
        ];
        let mut reader = WSHeaderReader::new();

        let (header, read_size) = reader.try_read_header(&data).unwrap_ready();
        assert_eq!(read_size, data.len());
        assert_eq!(header.fin, 1);
        assert_eq!(header.opcode, WSOpcode::Text);
        assert_eq!(header.payload_len, 0x0123456789abcdefusize);
        assert_eq!(header.masking_key, Some(0x12345678u32.to_be_bytes()));
    }

    #[test]
    fn test_header_reader_try_read_header_valid_partial() {
        let data = [
            0b10000001, 0xFF, 0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0x12, 0x34, 0x56, 0x78,
        ];
        let mut reader = WSHeaderReader::new();

        let mut it = data.into_iter();
        for _ in 0..data.len() - 1 {
            let byte = [it.next().unwrap()];
            let res = reader.try_read_header(&byte);
            if res.is_ready() {
                break;
            }
            let read_bytes = res.unwrap_pending();
            assert_eq!(read_bytes, 1);
        }

        // Now read the last byte
        let byte = [it.next().unwrap()];
        let (header, read_size) = reader.try_read_header(&byte).unwrap_ready();

        assert_eq!(read_size, data.len());
        assert_eq!(header.fin, 1);
        assert_eq!(header.opcode, WSOpcode::Text);
        assert_eq!(header.payload_len, 0x0123456789abcdefusize);
        assert_eq!(header.masking_key, Some(0x12345678u32.to_be_bytes()));
    }

    #[test]
    fn test_read_real_ws_packet_with_ws_stream_reader() {
        let mut src_buf = &REAL_WS_PACKET[..];

        let mut header_reader = WSHeaderReader::new();
        let (header, read_size) = header_reader.try_read_header(src_buf).unwrap_ready();
        assert_eq!(read_size, REAL_WS_PACKET_HEADER_SIZE);
        assert_eq!(header.fin, REAL_WS_PACKET_FIN);
        assert_eq!(header.opcode, WSOpcode::Text);
        assert_eq!(header.payload_len, REAL_WS_PACKET_PAYLOAD_SIZE);
        assert_eq!(
            header.masking_key.unwrap().as_slice(),
            REAL_WS_PACKET_MASKING_KEY.as_slice()
        );
        // Remove consumed data from src_buf
        src_buf = &src_buf[read_size..];

        let mut reader = WSPayloadReader::from_header(&header);
        assert_eq!(reader.payload_len(), REAL_WS_PACKET_PAYLOAD_SIZE);
        assert_eq!(reader.read_bytes(), 0);
        assert_eq!(reader.read_bytes_remaining(), REAL_WS_PACKET_PAYLOAD_SIZE);
        assert!(reader.has_masking());
        assert_eq!(
            reader.masking_key().unwrap().as_slice(),
            REAL_WS_PACKET_MASKING_KEY.as_slice()
        );

        let mut payload_buf = [0u8; REAL_WS_PACKET_PAYLOAD_SIZE];
        payload_buf.copy_from_slice(src_buf);
        let decoded_size = reader.decode_payload_in_place(&mut payload_buf);
        assert!(reader.is_complete());
        assert_eq!(decoded_size, REAL_WS_PACKET_PAYLOAD_SIZE);
        assert_eq!(decoded_size, reader.payload_len());
        assert_eq!(reader.read_bytes(), REAL_WS_PACKET_PAYLOAD_SIZE);
        assert_eq!(reader.read_bytes_remaining(), 0);

        let expected_payload: [u8; REAL_WS_PACKET_PAYLOAD_SIZE] = *b"Hello from client";

        assert_eq!(payload_buf, expected_payload);
    }

    #[test]
    fn test_consume_payload() {
        let header = WSFrameHeader {
            fin: 1,
            opcode: WSOpcode::Binary,
            payload_len: 10,
            masking_key: None,
        };
        let mut reader = WSPayloadReader::from_header(&header);

        // Consume 4 bytes
        let consumed = reader.consume_payload(4);
        assert_eq!(consumed, 4);
        assert_eq!(reader.read_bytes(), 4);
        assert_eq!(reader.read_bytes_remaining(), 6);

        // Consume more than remaining bytes
        let consumed = reader.consume_payload(10);
        assert_eq!(consumed, 6);
        assert_eq!(reader.read_bytes(), 10);
        assert_eq!(reader.read_bytes_remaining(), 0);

        // Consume when already complete
        let consumed = reader.consume_payload(5);
        assert_eq!(consumed, 0);
        assert_eq!(reader.read_bytes(), 10);
        assert_eq!(reader.read_bytes_remaining(), 0);
    }

    #[test]
    fn test_consume_all() {
        let header = WSFrameHeader {
            fin: 1,
            opcode: WSOpcode::Binary,
            payload_len: 10,
            masking_key: None,
        };
        let mut reader = WSPayloadReader::from_header(&header);
        let consumed = reader.consume_all();
        assert_eq!(consumed, 10);
        assert_eq!(reader.read_bytes(), 10);
        assert_eq!(reader.read_bytes_remaining(), 0);
    }
}
