#![allow(dead_code)]

/// Real WebSocket packet for testing.
/// 6 bytes header
/// fin=1, opcode=Text, masked=1, payload_len=17
/// masking_key=[0b01101000u8, 0b00010010u8, 0b11110001u8, 0b00110110u8]
/// payload=b"Hello from client"
pub(crate) const REAL_WS_PACKET: [u8; 23] = [
    0b10000001, 0b10010001, 0b01101000, 0b00010010, 0b11110001, 0b00110110, 0b00100000, 0b01110111, 0b10011101,
    0b01011010, 0b00000111, 0b00110010, 0b10010111, 0b01000100, 0b00000111, 0b01111111, 0b11010001, 0b01010101,
    0b00000100, 0b01111011, 0b10010100, 0b01011000, 0b00011100,
];

pub(crate) const REAL_WS_PACKET_HEADER_SIZE: usize = 6;
pub(crate) const REAL_WS_PACKET_PAYLOAD_SIZE: usize = 17;
pub(crate) const REAL_WS_PACKET_MASKING_KEY: [u8; 4] = [0b01101000, 0b00010010, 0b11110001, 0b00110110];
pub(crate) const REAL_WS_PACKET_PAYLOAD: [u8; REAL_WS_PACKET_PAYLOAD_SIZE] = *b"Hello from client";
pub(crate) const REAL_WS_PACKET_FIN: u8 = 1;
pub(crate) const REAL_WS_PACKET_OPCODE: u8 = 0x1; // Text frame
pub(crate) const REAL_WS_PACKET_MASKED: u8 = 1;
