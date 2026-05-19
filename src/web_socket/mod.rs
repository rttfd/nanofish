/// WebSocket protocol header
mod header;

/// WebSocket header reader module.
mod header_reader;

/// WebSocket header writer module.
mod header_writer;

/// Websocket implementation.
#[allow(clippy::module_inception)]
mod web_socket;

/// Test utilities for WebSocket.
#[cfg(test)]
pub mod test_utils;

pub use web_socket::{WebSocket, WebSocketError, WebSocketState};

// Re-export the traits to make implementations visible
pub use embedded_io_async::{
    Error as WebSocketIoError, Read as WebSocketRead, ReadReady as WebSocketReadReady, Write as WebSocketWrite,
    WriteReady as WebSocketWriteReady,
};

// Tests
#[cfg(test)]
mod tests {
    use super::test_utils::*;
    use crate::web_socket::header::*;
    use crate::web_socket::header_reader::*;
    use crate::web_socket::header_writer::*;

    #[test]
    fn test_read_and_write_packet() {
        let mut src_buf = &REAL_WS_PACKET[..];
        let mut decoded_payload_buf = [0u8; REAL_WS_PACKET_PAYLOAD_SIZE];
        let mut encoded_frame_buf = [0u8; REAL_WS_PACKET.len()];

        let mut header_reader = WSHeaderReader::new();
        let (header, decoded_header_size) = header_reader.try_read_header(src_buf).unwrap_ready();
        src_buf = &src_buf[decoded_header_size..];
        assert_eq!(decoded_header_size, REAL_WS_PACKET_HEADER_SIZE);
        assert_eq!(header.payload_len, REAL_WS_PACKET_PAYLOAD_SIZE);

        decoded_payload_buf.clone_from_slice(src_buf);

        let mut reader = WSPayloadReader::from_header(&header);
        let decoded_payload_size = reader.decode_payload_in_place(&mut decoded_payload_buf);
        assert!(reader.is_complete());
        assert_eq!(decoded_payload_size, REAL_WS_PACKET_PAYLOAD_SIZE);
        assert_eq!(decoded_payload_size, reader.payload_len());
        assert_eq!(
            &decoded_payload_buf[..decoded_payload_size],
            &REAL_WS_PACKET_PAYLOAD[..]
        );

        let mut header_bytes = [0u8; MAX_WS_FRAME_HEADER_SIZE];
        let (mut writer, encoded_header_size) = WSEncodeWriter::encode_header(
            reader.payload_len(),
            &mut header_bytes,
            header.opcode,
            header.fin,
            reader.masking_key().unwrap().clone(),
        );
        encoded_frame_buf[..encoded_header_size].copy_from_slice(&header_bytes[..encoded_header_size]);
        encoded_frame_buf[encoded_header_size..encoded_header_size + decoded_payload_size]
            .copy_from_slice(&decoded_payload_buf[..decoded_payload_size]);

        let encoded_payload_size = writer
            .encode_payload_in_place(&mut encoded_frame_buf[encoded_header_size..])
            .unwrap();

        assert_eq!(encoded_header_size, decoded_header_size);
        assert_eq!(encoded_payload_size, decoded_payload_size);

        assert_eq!(
            &encoded_frame_buf[..encoded_header_size + encoded_payload_size],
            &REAL_WS_PACKET[..]
        );
    }
}
