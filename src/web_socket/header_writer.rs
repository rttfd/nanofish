#![allow(dead_code)]

use crate::web_socket::header::*;

pub struct WSEncodeWriter {
    available_space: usize,
    idx: usize,
    masking_key: MaskKey,
}

impl WSEncodeWriter {
    pub fn encode_header(
        payload_len: usize,
        out_buf: &mut [u8; MAX_WS_FRAME_HEADER_SIZE],
        opcode: WSOpcode,
        fin: u8,
        masking_key: [u8; 4],
    ) -> (Self, usize) {
        let header_size: usize = write_frame_header_with_mask_key(payload_len, out_buf, opcode, fin, masking_key);

        let result = Self {
            available_space: payload_len,
            idx: 0,
            masking_key,
        };

        (result, header_size)
    }

    /// Encodes payload in place within the buffer with provided masking key.
    /// The payload data in the buffer will be modified by XORing it with the masking key.
    /// This function expect overall sum of payload bytes across all calls to be less than
    /// or equal to the payload length specified during header encoding.
    ///
    /// ## Returns
    /// The number of bytes written to the payload_dst, or Err(()) if the payload_src length
    /// is greater than the allocated payload length that left.
    ///
    /// ## Errors
    /// Returns Err(()) if the payload_src length is greater than the allocated payload length that left.
    ///
    pub fn encode_payload_in_place(&mut self, payload_buf: &mut [u8]) -> Result<usize, ()> {
        if payload_buf.len() > self.available_space {
            return Err(());
        }

        let buf = payload_buf.iter_mut();
        let masking_key = self.masking_key;
        let mut transferred: usize = 0;

        for buf_byte in buf {
            let j = self.idx % masking_key.len();
            let key_byte = masking_key[j];
            *buf_byte ^= key_byte;
            self.idx += 1;
            transferred += 1;
        }

        self.available_space -= transferred;
        Ok(transferred)
    }
}

// Tests
#[cfg(test)]
mod tests {
    use super::*;
    use crate::web_socket::test_utils::*;

    #[test]
    fn test_ws_encode_writer() {
        const PAYLOAD: &[u8] = b"Hello, WebSocket!";
        const MASKING_KEY: [u8; 4] = [0x12, 0x34, 0x56, 0x78];
        const EXPECTED_WS_PACKET: &[u8] = &[
            0x81, 0x91, 0x12, 0x34, 0x56, 0x78, 0x5A, 0x51, 0x3A, 0x14, 0x7D, 0x18, 0x76, 0x2F, 0x77, 0x56, 0x05, 0x17,
            0x71, 0x5F, 0x33, 0x0C, 0x33,
        ]; // Masked frame with "Hello, WebSocket!" payload

        let payload: &[u8] = PAYLOAD; //17 bytes
        let masking_key: [u8; 4] = MASKING_KEY;
        let mut buf = [0u8; MAX_WS_FRAME_HEADER_SIZE + PAYLOAD.len()];
        let mut header_buffer = [0u8; MAX_WS_FRAME_HEADER_SIZE];

        let (mut writer, header_size) =
            WSEncodeWriter::encode_header(payload.len(), &mut header_buffer, WSOpcode::Text, 1, masking_key);
        assert_eq!(header_size, MIN_WS_FRAME_HEADER_SIZE + WS_MASKING_KEY_LEN);

        buf[..header_size].copy_from_slice(&header_buffer[..header_size]);

        buf[header_size..header_size + payload.len()].copy_from_slice(payload);
        let encoded_payload_size = writer
            .encode_payload_in_place(&mut buf[header_size..header_size + payload.len()])
            .unwrap();
        assert_eq!(encoded_payload_size, payload.len());
        assert_eq!(&buf[..header_size + encoded_payload_size], EXPECTED_WS_PACKET);
    }

    #[test]
    fn test_ws_encode_writer_encode_real_frame() {
        let mut buf = [0u8; REAL_WS_PACKET.len()];
        let mut header_buffer = [0u8; MAX_WS_FRAME_HEADER_SIZE];

        let (mut writer, header_size) = WSEncodeWriter::encode_header(
            REAL_WS_PACKET_PAYLOAD.len(),
            &mut header_buffer,
            WSOpcode::Text,
            REAL_WS_PACKET_FIN,
            REAL_WS_PACKET_MASKING_KEY,
        );
        assert_eq!(header_size, REAL_WS_PACKET_HEADER_SIZE);

        buf[..header_size].copy_from_slice(&header_buffer[..header_size]);

        buf[header_size..header_size + REAL_WS_PACKET_PAYLOAD.len()].copy_from_slice(&REAL_WS_PACKET_PAYLOAD);
        let encoded_payload_size = writer
            .encode_payload_in_place(&mut buf[header_size..header_size + REAL_WS_PACKET_PAYLOAD.len()])
            .unwrap();
        assert_eq!(encoded_payload_size, REAL_WS_PACKET_PAYLOAD.len());
        assert_eq!(&buf[..header_size + encoded_payload_size], REAL_WS_PACKET);
    }
}
