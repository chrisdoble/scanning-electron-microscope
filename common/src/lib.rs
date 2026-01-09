#![no_std]

use core::{error, fmt};
#[cfg(feature = "defmt")]
use defmt::*;

// The maximum USB packet size that we can read or write. For full-speed devices
// (like the Raspberry Pi Pico 2), this must be 8, 16, 32, or 64[1].
//
// 1: https://docs.embassy.dev/embassy-usb/git/default/class/cdc_acm/struct.CdcAcmClass.html#method.new
pub const USB_MAX_PACKET_SIZE: u8 = 64;

/// An error that occurred on the vacuum system controller.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(Format))]
pub enum Error {
    /// A command was missing a destination prefix.
    ///
    /// Each command must include a three letter destination prefix followed by
    /// a colon (e.g. "ADC:?GA1\r").
    CommandMissingDestination,

    /// A command wasn't valid UTF-8.
    CommandNotUtf8,

    /// The USB host took too long to send a command.
    CommandTimeout,

    /// A command was too long.
    ///
    /// The maximum length is determined by the size of the command buffer.
    CommandTooLong,

    /// A command was too short.
    ///
    /// Each command must include a three letter destination prefix, a colon, at
    /// least one letter, and a terminator, making a minimum of 6 characters.
    CommandTooShort,

    /// The USB host disconnected.
    Disconnected,

    /// A response wasn't valid UTF-8.
    ResponseNotUtf8,

    /// One of the devices took too long to send a response.
    ResponseTimeout,

    /// One of the devices send a response that was too long.
    ///
    /// The maximum length is determined by the size of the receive buffer.
    ResponseTooLong,

    /// An unknown error.
    Unknown,

    /// A command contained an unknown destination prefix.
    UnknownDestination,
}

impl Error {
    /// Converts the error into a response that can be sent to the USB host.
    pub fn to_response(&self) -> &[u8] {
        match self {
            Error::CommandMissingDestination => "ERR:COMMAND_MISSING_DESTINATION\r\n",
            Error::CommandNotUtf8 => "ERRO:COMMAND_NOT_UTF8\r\n",
            Error::CommandTimeout => "ERR:COMMAND_TIMEOUT\r\n",
            Error::CommandTooLong => "ERR:COMMAND_TOO_LONG\r\n",
            Error::CommandTooShort => "ERR:COMMAND_TOO_SHORT\r\n",
            Error::Disconnected => "ERR:DISCONNECTED\r\n",
            Error::ResponseNotUtf8 => "ERR:RESPONSE_NOT_UTF8\r\n",
            Error::ResponseTimeout => "ERR:RESPONSE_TIMEOUT\r\n",
            Error::ResponseTooLong => "ERR:RESPONSE_TOO_LONG\r\n",
            Error::Unknown => "ERR:UNKNOWN\r\n",
            Error::UnknownDestination => "ERR:UNKNOWN_DESTINATION\r\n",
        }
        .as_bytes()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // TODO
        core::write!(f, "")
    }
}

impl error::Error for Error {}
