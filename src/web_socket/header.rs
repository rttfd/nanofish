use modular_bitfield::prelude::*;

pub const MIN_WS_FRAME_HEADER_SIZE: usize = 2;
pub(crate) const WS_EXTENDEDPAYLOAD_LEN_SHORT: usize = 2;
pub(crate) const WS_EXTENDEDPAYLOAD_LEN_LONG: usize = 8;
pub(crate) const WS_MASKING_KEY_LEN: usize = 4;
pub const MAX_WS_FRAME_HEADER_SIZE: usize = MIN_WS_FRAME_HEADER_SIZE + WS_EXTENDEDPAYLOAD_LEN_LONG + WS_MASKING_KEY_LEN; // 2 + 8 + 4

pub type MaskKey = [u8; WS_MASKING_KEY_LEN];

pub type WSRequiredSizeHint = usize;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WebSocketProtoError {
    /// Not enough data available to read the complete frame header.
    /// Contains the number hint of bytes needed.
    NotEnoughData(WSRequiredSizeHint),
    InvalidFrame,
}

#[derive(Specifier, Debug, Clone, Copy, PartialEq)]
#[bits = 4]
pub enum WSOpcode {
    ContinuationFrame = 0x0,
    Text = 0x1,
    Binary = 0x2,
    Close = 0x8,
    Ping = 0x9,
    Pong = 0xA,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WSFrameHeader {
    pub payload_len: usize,
    pub masking_key: Option<MaskKey>,
    pub opcode: WSOpcode,
    pub fin: u8,
}

pub fn read_frame_header(buffer: &[u8]) -> Result<(WSFrameHeader, usize), WebSocketProtoError> {
    let mut expected_size = MIN_WS_FRAME_HEADER_SIZE;
    if buffer.len() < expected_size {
        return Err(WebSocketProtoError::NotEnoughData(
            expected_size, // Need at least 2 bytes
        ));
    };

    // Safety: We have already checked that the buffer length is at least MIN_WS_FRAME_HEADER_SIZE
    let header_bytes = unsafe { *buffer.first_chunk::<MIN_WS_FRAME_HEADER_SIZE>().unwrap_unchecked() };
    let header = WebSocketFrameHeaderPacked::from_bytes(header_bytes);
    let opcode = header.opcode_or_err().map_err(|_| WebSocketProtoError::InvalidFrame)?;

    expected_size += if header.mask() == 1 { WS_MASKING_KEY_LEN } else { 0 };

    let payload_len = match header.payload_len() {
        len @ 0..=125 => {
            if buffer.len() < expected_size {
                return Err(WebSocketProtoError::NotEnoughData(
                    expected_size, // Need at least 2 bytes + 2 bytes of masking key if present
                ));
            }

            len as usize
        }
        126 => {
            expected_size += WS_EXTENDEDPAYLOAD_LEN_SHORT;

            if buffer.len() < expected_size {
                return Err(WebSocketProtoError::NotEnoughData(
                    expected_size, // Need at least 4 bytes + 2 bytes of masking key if present
                ));
            }

            // The value is stored in network byte order (big-endian)
            const VALUE_START: usize = MIN_WS_FRAME_HEADER_SIZE;
            const VALUE_END: usize = MIN_WS_FRAME_HEADER_SIZE + WS_EXTENDEDPAYLOAD_LEN_SHORT;
            let extended_length: u16 = u16::from_be_bytes(buffer[VALUE_START..VALUE_END].try_into().unwrap());
            extended_length as usize
        }
        127 => {
            expected_size += WS_EXTENDEDPAYLOAD_LEN_LONG;

            if buffer.len() < expected_size {
                return Err(WebSocketProtoError::NotEnoughData(
                    expected_size, // Need at least 4 bytes + 2 bytes of masking key if present
                ));
            }
            // The value is stored in network byte order (big-endian)
            const VALUE_START: usize = MIN_WS_FRAME_HEADER_SIZE;
            const VALUE_END: usize = MIN_WS_FRAME_HEADER_SIZE + WS_EXTENDEDPAYLOAD_LEN_LONG;
            let extended_length = u64::from_be_bytes(buffer[VALUE_START..VALUE_END].try_into().unwrap());
            if extended_length >> 63 != 0 {
                // Most significant bit must be 0
                return Err(WebSocketProtoError::InvalidFrame);
            }
            extended_length as usize
        }
        _ => return Err(WebSocketProtoError::InvalidFrame),
    };

    let masking_key = if header.mask() == 1 {
        MaskKey::try_from(&buffer[expected_size - WS_MASKING_KEY_LEN..expected_size]).ok()
    } else {
        None
    };

    let reading = WSFrameHeader {
        payload_len,
        masking_key,
        opcode,
        fin: header.fin(),
    };

    Ok((reading, expected_size))
}

/// Writes a WebSocket frame header into the provided buffer.
fn write_frame_header_impl(
    payload_len: usize,
    buffer: &mut [u8; MAX_WS_FRAME_HEADER_SIZE],
    opcode: WSOpcode,
    fin: u8,
    mask_bit: u8,
) -> usize {
    let mut buf = buffer.as_mut();

    let mut header = WebSocketFrameHeaderPacked::new();
    header.set_fin(fin);
    header.set_opcode(opcode);
    header.set_mask(mask_bit);

    let pos: usize = if payload_len <= 125 {
        header.set_payload_len(payload_len as u8);
        buf[..MIN_WS_FRAME_HEADER_SIZE].copy_from_slice(&header.into_bytes());
        MIN_WS_FRAME_HEADER_SIZE
    } else if payload_len <= 65535 {
        header.set_payload_len(126);
        buf[..MIN_WS_FRAME_HEADER_SIZE].copy_from_slice(&header.into_bytes());
        buf = &mut buf[MIN_WS_FRAME_HEADER_SIZE..];

        let payload_len_sort = payload_len as u16;
        buf[..WS_EXTENDEDPAYLOAD_LEN_SHORT].copy_from_slice(&payload_len_sort.to_be_bytes());
        MIN_WS_FRAME_HEADER_SIZE + WS_EXTENDEDPAYLOAD_LEN_SHORT
    } else {
        header.set_payload_len(127);
        buf[..MIN_WS_FRAME_HEADER_SIZE].copy_from_slice(&header.into_bytes());
        buf = &mut buf[MIN_WS_FRAME_HEADER_SIZE..];

        buf[..WS_EXTENDEDPAYLOAD_LEN_LONG].copy_from_slice(&payload_len.to_be_bytes());
        MIN_WS_FRAME_HEADER_SIZE + WS_EXTENDEDPAYLOAD_LEN_LONG
    };

    debug_assert!(pos <= MAX_WS_FRAME_HEADER_SIZE);
    pos
}

/// Writes a WebSocket frame header into the provided buffer.
#[inline(always)]
pub fn write_frame_header(
    payload_len: usize,
    buffer: &mut [u8; MAX_WS_FRAME_HEADER_SIZE],
    opcode: WSOpcode,
    fin: u8,
) -> usize {
    write_frame_header_impl(payload_len, buffer, opcode, fin, 0)
}

/// Writes a WebSocket frame header into the provided buffer.
pub(crate) fn write_frame_header_with_mask_key(
    payload_len: usize,
    buffer: &mut [u8; MAX_WS_FRAME_HEADER_SIZE],
    opcode: WSOpcode,
    fin: u8,
    masking_key: MaskKey,
) -> usize {
    let mut pos = write_frame_header_impl(payload_len, buffer, opcode, fin, 1);

    buffer[pos..pos + WS_MASKING_KEY_LEN].copy_from_slice(&masking_key);
    pos += WS_MASKING_KEY_LEN;

    debug_assert!(pos <= MAX_WS_FRAME_HEADER_SIZE);
    pos
}

#[bitfield]
struct WebSocketFrameHeaderPacked {
    // Byte 0
    #[bits = 4]
    #[allow(dead_code)]
    opcode: WSOpcode,
    #[skip]
    __: B3,
    fin: B1,

    // Byte 1
    payload_len: B7,
    mask: B1,
}

// Tests
#[cfg(test)]
mod tests {
    use super::*;
    const REAL_WS_PACKET: [u8; 23] = [
        0b10000001, 0b10010001, 0b01101000, 0b00010010, 0b11110001, 0b00110110, 0b00100000, 0b01110111, 0b10011101,
        0b01011010, 0b00000111, 0b00110010, 0b10010111, 0b01000100, 0b00000111, 0b01111111, 0b11010001, 0b01010101,
        0b00000100, 0b01111011, 0b10010100, 0b01011000, 0b00011100,
    ];

    #[test]
    fn test_read_real_ws_packet() {
        let (reading, read_size) = read_frame_header(&REAL_WS_PACKET).unwrap();
        assert_eq!(read_size, 6);
        assert_eq!(reading.fin, 1);
        assert_eq!(reading.opcode, WSOpcode::Text);
        assert_eq!(reading.payload_len, 17);
        assert_eq!(
            reading.masking_key,
            Some(0b01101000_00010010_11110001_00110110u32.to_be_bytes())
        );
    }

    // ContinuationFrame = 0x0,
    // Text = 0x1,
    // Binary = 0x2,
    // Close = 0x8,
    // Ping = 0x9,
    // Pong = 0xA,
    #[test]
    fn test_header_opcode_decoding_continuation_frame_h00() {
        let (reading, read_size) = read_frame_header(&[0b0000_0000, 0b0000_0000]).unwrap();
        assert_eq!(read_size, MIN_WS_FRAME_HEADER_SIZE);
        assert_eq!(reading.fin, 0);
        assert_eq!(reading.opcode, WSOpcode::ContinuationFrame);
    }

    #[test]
    fn test_header_opcode_decoding_text_h01() {
        let (reading, read_size) = read_frame_header(&[0b0000_0001, 0b0000_0000]).unwrap();
        assert_eq!(read_size, MIN_WS_FRAME_HEADER_SIZE);
        assert_eq!(reading.fin, 0);
        assert_eq!(reading.opcode, WSOpcode::Text);
    }

    #[test]
    fn test_header_opcode_decoding_binary_h02() {
        let (reading, read_size) = read_frame_header(&[0b0000_0010, 0b0000_0000]).unwrap();
        assert_eq!(read_size, MIN_WS_FRAME_HEADER_SIZE);
        assert_eq!(reading.fin, 0);
        assert_eq!(reading.opcode, WSOpcode::Binary);
    }

    #[test]
    fn test_header_opcode_decoding_close_h08() {
        let (reading, read_size) = read_frame_header(&[0b0000_1000, 0b0000_0000]).unwrap();
        assert_eq!(read_size, MIN_WS_FRAME_HEADER_SIZE);
        assert_eq!(reading.fin, 0);
        assert_eq!(reading.opcode, WSOpcode::Close);
    }

    #[test]
    fn test_header_opcode_decoding_ping_h09() {
        let (reading, read_size) = read_frame_header(&[0b0000_1001, 0b0000_0000]).unwrap();
        assert_eq!(read_size, MIN_WS_FRAME_HEADER_SIZE);
        assert_eq!(reading.fin, 0);
        assert_eq!(reading.opcode, WSOpcode::Ping);
    }

    #[test]
    fn test_header_opcode_decoding_pong_h0a() {
        let (reading, read_size) = read_frame_header(&[0b0000_1010, 0b0000_0000]).unwrap();
        assert_eq!(read_size, MIN_WS_FRAME_HEADER_SIZE);
        assert_eq!(reading.fin, 0);
        assert_eq!(reading.opcode, WSOpcode::Pong);
    }

    #[test]
    fn test_header_opcode_decoding_invalid() {
        let Err(e) = read_frame_header(&[0b0000_0011, 0b0000_0000]) else {
            panic!("Expected error for invalid opcode");
        };
        assert_eq!(e, WebSocketProtoError::InvalidFrame);
    }

    #[test]
    fn test_header_fin_decoding_1() {
        let (reading, read_size) = read_frame_header(&[0b1000_0000, 0b0000_0000]).unwrap();
        assert_eq!(read_size, MIN_WS_FRAME_HEADER_SIZE);
        assert_eq!(reading.fin, 1);
    }

    #[test]
    fn test_header_fin_decoding_0() {
        let (reading, read_size) = read_frame_header(&[0b0000_0000, 0b0000_0000]).unwrap();
        assert_eq!(read_size, MIN_WS_FRAME_HEADER_SIZE);
        assert_eq!(reading.fin, 0);
    }

    #[test]
    fn test_header_masking_key_decoding_none() {
        let (reading, read_size) = read_frame_header(&[0b0000_0000, 0b0000_0000]).unwrap();
        assert_eq!(read_size, MIN_WS_FRAME_HEADER_SIZE);
        assert_eq!(reading.masking_key, None);
    }

    #[test]
    fn test_header_masking_key_decoding_some() {
        let (reading, read_size) = read_frame_header(&[0b0000_0000, 0b1000_0000, 0x12, 0x34, 0x56, 0x78]).unwrap();
        assert_eq!(read_size, MIN_WS_FRAME_HEADER_SIZE + WS_MASKING_KEY_LEN);
        assert_eq!(reading.masking_key.unwrap(), 0x12345678u32.to_be_bytes());
    }

    #[test]
    fn test_header_payload_len_decoding_le125() {
        let (reading, read_size) = read_frame_header(&[0b0000_0000, 0x7D]).unwrap();
        assert_eq!(read_size, MIN_WS_FRAME_HEADER_SIZE);
        assert_eq!(reading.payload_len, 0x7Dusize);
    }

    #[test]
    fn test_header_payload_len_decoding_short() {
        let (reading, read_size) = read_frame_header(&[0b0000_0000, 0x7E, 0x01, 0x2C]).unwrap();
        assert_eq!(read_size, MIN_WS_FRAME_HEADER_SIZE + WS_EXTENDEDPAYLOAD_LEN_SHORT);
        assert_eq!(reading.payload_len, 300usize);
    }

    #[test]
    fn test_header_payload_len_decoding_long() {
        let (reading, read_size) =
            read_frame_header(&[0b0000_0000, 0x7F, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef]).unwrap();
        assert_eq!(read_size, MIN_WS_FRAME_HEADER_SIZE + WS_EXTENDEDPAYLOAD_LEN_LONG);
        assert_eq!(reading.payload_len, 0x0123456789abcdefusize);
    }

    #[test]
    fn test_header_payload_short_len_not_enough_data() {
        let data = [0b0000_0000, 0x7E, 0x01 /*, 0x2C */];
        let Err(e) = read_frame_header(&data) else {
            panic!("Expected error for invalid frame");
        };
        assert_eq!(
            e,
            WebSocketProtoError::NotEnoughData(MIN_WS_FRAME_HEADER_SIZE + WS_EXTENDEDPAYLOAD_LEN_SHORT)
        );
    }

    #[test]
    fn test_header_payload_long_len_not_enough_data() {
        let data = [
            0b0000_0000,
            0x7F,
            0x01,
            0x23,
            0x45,
            0x67,
            0x89,
            0xab,
            0xcd, /*0xef*/
        ];
        let Err(e) = read_frame_header(&data) else {
            panic!("Expected error for invalid frame");
        };
        assert_eq!(
            e,
            WebSocketProtoError::NotEnoughData(MIN_WS_FRAME_HEADER_SIZE + WS_EXTENDEDPAYLOAD_LEN_LONG)
        );
    }

    #[test]
    fn test_header_payload_len_invalid_value() {
        let Err(e) = read_frame_header(&[
            0b0000_0000,
            0x7F,
            0x81, // Most significant bit set
            0x23,
            0x45,
            0x67,
            0x89,
            0xab,
            0xcd,
            0xef,
        ]) else {
            panic!("Expected error for invalid frame");
        };
        assert_eq!(e, WebSocketProtoError::InvalidFrame);
    }

    #[test]
    fn test_read_frame_header_returns_not_enough_data_when_size_0() {
        let Err(e) = read_frame_header(&[0b0000_0000]) else {
            panic!("Expected NotEnoughData error");
        };
        assert_eq!(e, WebSocketProtoError::NotEnoughData(MIN_WS_FRAME_HEADER_SIZE));
    }

    #[test]
    fn test_read_frame_header_returns_not_enough_data_when_size_1() {
        let Err(e) = read_frame_header(&[]) else {
            panic!("Expected NotEnoughData error");
        };
        assert_eq!(e, WebSocketProtoError::NotEnoughData(MIN_WS_FRAME_HEADER_SIZE));
    }

    #[test]
    fn test_read_frame_header_returns_not_enough_data_when_masking_key_not_enough() {
        let Err(e) = read_frame_header(&[0b0000_0000, 0b1000_0000, 0x12, 0x34, 0x56 /*0x78*/]) else {
            panic!("Expected NotEnoughData error");
        };
        assert_eq!(
            e,
            WebSocketProtoError::NotEnoughData(MIN_WS_FRAME_HEADER_SIZE + WS_MASKING_KEY_LEN)
        );
    }

    #[test]
    fn test_write_and_read_frame_header() {
        let mut buffer = [0u8; MAX_WS_FRAME_HEADER_SIZE];
        let fin = 1;
        let opcode = WSOpcode::Text;
        let payload_len = 300;
        let masking_key = 0xAABBCCDDu32.to_be_bytes();

        let header_size = write_frame_header_with_mask_key(payload_len, &mut buffer, opcode, fin, masking_key);

        let (reading, read_size) = read_frame_header(&buffer).unwrap();

        assert_eq!(header_size, read_size);
        assert_eq!(reading.fin, fin);
        assert_eq!(reading.opcode as u8, opcode as u8);
        assert_eq!(reading.payload_len, payload_len);
        assert_eq!(reading.masking_key, Some(masking_key));
    }
}
