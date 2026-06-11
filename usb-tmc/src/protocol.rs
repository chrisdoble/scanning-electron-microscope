//! Encoding and decoding of the USBTMC bulk transfer message headers used by
//! `DEV_DEP_MSG_OUT`, `REQUEST_DEV_DEP_MSG_IN`, and `DEV_DEP_MSG_IN` (USBTMC
//! 1.00, section 3).

use crate::Error;

/// The size in bytes of a USBTMC bulk transfer header.
pub const HEADER_LEN: usize = 12;

/// `bInterfaceClass` for a USBTMC interface.
pub const USBTMC_INTERFACE_CLASS: u8 = 0xfe;

/// `bInterfaceSubClass` for a USBTMC interface.
pub const USBTMC_INTERFACE_SUBCLASS: u8 = 0x03;

const DEV_DEP_MSG_OUT: u8 = 1;
const REQUEST_DEV_DEP_MSG_IN: u8 = 2;
const DEV_DEP_MSG_IN: u8 = 2;

/// Bit 0 of `bmTransferAttributes` (header byte 8). Its meaning depends on
/// the message type: for `DEV_DEP_MSG_OUT`/`DEV_DEP_MSG_IN` it's `EOM`; for
/// `REQUEST_DEV_DEP_MSG_IN` it's `TermCharEnabled`.
const TRANSFER_ATTRIBUTES_BIT0: u8 = 1 << 0;

/// The number of padding bytes needed to bring `len` up to a multiple of 4,
/// as required after the data of a `DEV_DEP_MSG_OUT` or `DEV_DEP_MSG_IN`
/// message.
pub fn padding_len(len: usize) -> usize {
    (4 - (len % 4)) % 4
}

/// Encodes a `DEV_DEP_MSG_OUT` header for a message of `transfer_size` bytes.
///
/// `eom` should be `true`, since this crate always sends a complete message
/// in a single `DEV_DEP_MSG_OUT` transfer.
pub fn encode_dev_dep_msg_out(b_tag: u8, transfer_size: u32, eom: bool) -> [u8; HEADER_LEN] {
    let bm_transfer_attributes = if eom { TRANSFER_ATTRIBUTES_BIT0 } else { 0 };
    encode_header(
        DEV_DEP_MSG_OUT,
        b_tag,
        transfer_size,
        bm_transfer_attributes,
    )
}

/// Encodes a `REQUEST_DEV_DEP_MSG_IN` header requesting up to `transfer_size`
/// bytes of response data, with `TermCharEnabled` left unset.
pub fn encode_request_dev_dep_msg_in(b_tag: u8, transfer_size: u32) -> [u8; HEADER_LEN] {
    encode_header(REQUEST_DEV_DEP_MSG_IN, b_tag, transfer_size, 0)
}

fn encode_header(
    msg_id: u8,
    b_tag: u8,
    transfer_size: u32,
    bm_transfer_attributes: u8,
) -> [u8; HEADER_LEN] {
    let mut header = [0u8; HEADER_LEN];
    header[0] = msg_id;
    header[1] = b_tag;
    header[2] = !b_tag;
    header[4..8].copy_from_slice(&transfer_size.to_le_bytes());
    header[8] = bm_transfer_attributes;
    header
}

/// A decoded `DEV_DEP_MSG_IN` header.
pub struct DevDepMsgIn {
    /// The number of payload bytes that follow the header (before any
    /// padding).
    pub transfer_size: u32,
    /// Whether this is the last transfer of the response.
    pub eom: bool,
}

/// Decodes and validates a `DEV_DEP_MSG_IN` header received in response to a
/// `REQUEST_DEV_DEP_MSG_IN` with the given `b_tag`.
pub fn decode_dev_dep_msg_in(
    header: &[u8; HEADER_LEN],
    expected_b_tag: u8,
) -> Result<DevDepMsgIn, Error> {
    let msg_id = header[0];
    let b_tag = header[1];
    let b_tag_inverse = header[2];

    if msg_id != DEV_DEP_MSG_IN {
        return Err(Error::Protocol(format!(
            "expected a DEV_DEP_MSG_IN header (MsgID {DEV_DEP_MSG_IN:#04x}), got MsgID {msg_id:#04x}"
        )));
    }
    if b_tag != expected_b_tag {
        return Err(Error::Protocol(format!(
            "expected bTag {expected_b_tag}, got {b_tag}"
        )));
    }
    if b_tag_inverse != !expected_b_tag {
        return Err(Error::Protocol(format!(
            "expected bTagInverse {:#04x}, got {b_tag_inverse:#04x}",
            !expected_b_tag
        )));
    }

    let transfer_size = u32::from_le_bytes(header[4..8].try_into().unwrap());
    let eom = header[8] & TRANSFER_ATTRIBUTES_BIT0 != 0;

    Ok(DevDepMsgIn { transfer_size, eom })
}

/// Tracks the `bTag` to use for the next bulk transfer.
///
/// `bTag` values cycle through 1..=255 (0 is reserved by the spec).
#[derive(Debug, Default)]
pub struct BTagSequence(u8);

impl BTagSequence {
    /// Returns the next `bTag` value, advancing the sequence.
    pub fn next(&mut self) -> u8 {
        self.0 = self.0 % 255 + 1;
        self.0
    }
}
