#![no_std]

use core::{error, fmt, str};
#[cfg(feature = "defmt")]
use defmt::*;

// The maximum USB packet size that we can read or write. For full-speed devices
// (like the Raspberry Pi Pico 2), this must be 8, 16, 32, or 64[1].
//
// 1: https://docs.embassy.dev/embassy-usb/git/default/class/cdc_acm/struct.CdcAcmClass.html#method.new
pub const USB_MAX_PACKET_SIZE: u8 = 64;

/// A vacuum system controller error.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(Format))]
pub enum ControllerError {
    /// A command was missing a destination prefix.
    ///
    /// Each command must include a three letter destination prefix followed by
    /// a colon (e.g. "ADC:?GA1\r").
    CommandMissingDestination,

    /// The USB host took too long to send a command.
    CommandTimeout,

    /// A command was too long.
    ///
    /// The maximum length is the length of the firmware's command buffer.
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
    /// The maximum length is the length of the firmware's response buffer.
    ResponseTooLong,

    /// An unknown error.
    Unknown,

    /// A command contained an unknown destination prefix.
    UnknownDestination,
}

impl error::Error for ControllerError {}

impl fmt::Display for ControllerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ControllerError::CommandMissingDestination => "COMMAND_MISSING_DESTINATION",
            ControllerError::CommandTimeout => "COMMAND_TIMEOUT",
            ControllerError::CommandTooLong => "COMMAND_TOO_LONG",
            ControllerError::CommandTooShort => "COMMAND_TOO_SHORT",
            ControllerError::Disconnected => "DISCONNECTED",
            ControllerError::ResponseNotUtf8 => "RESPONSE_NOT_UTF8",
            ControllerError::ResponseTimeout => "RESPONSE_TIMEOUT",
            ControllerError::ResponseTooLong => "RESPONSE_TOO_LONG",
            ControllerError::Unknown => "UNKNOWN",
            ControllerError::UnknownDestination => "UNKNOWN_DESTINATION",
        })
    }
}

impl str::FromStr for ControllerError {
    type Err = ControllerError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "COMMAND_MISSING_DESTINATION" => ControllerError::CommandMissingDestination,
            "COMMAND_TIMEOUT" => ControllerError::CommandTimeout,
            "COMMAND_TOO_LONG" => ControllerError::CommandTooLong,
            "COMMAND_TOO_SHORT" => ControllerError::CommandTooShort,
            "DISCONNECTED" => ControllerError::Disconnected,
            "RESPONSE_NOT_UTF8" => ControllerError::ResponseNotUtf8,
            "RESPONSE_TIMEOUT" => ControllerError::ResponseTimeout,
            "RESPONSE_TOO_LONG" => ControllerError::ResponseTooLong,
            "UNKNOWN_DESTINATION" => ControllerError::UnknownDestination,
            _ => ControllerError::Unknown,
        })
    }
}
