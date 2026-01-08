#![no_std]

#[cfg(feature = "defmt")]
use defmt::*;

/// An error that occurred on the vacuum system controller.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(Format))]
pub enum Error {
    /// One of the devices send a response that was too long.
    ///
    /// The maximum length is determined by the size of the receive buffer.
    UartResponseTooLong,

    /// One of the devices took too long to send a response.
    UartTimeout,

    /// An unknown UART error (communicating with a device).
    UartUnknown,

    /// The USB host sent a command that was missing a destination prefix.
    ///
    /// Each command must include a three letter destination prefix followed by
    /// a colon (e.g. "ADC:?GA1\r").
    UsbCommandMissingDestination,

    /// The USB host sent a command that was too long.
    ///
    /// The maximum length is determined by the size of the command buffer.
    UsbCommandTooLong,

    /// The USB host sent a command that was too short.
    ///
    /// Each command must include a three letter destination prefix, a colon, at
    /// least one letter, and a terminator, making a minimum of 6 characters.
    UsbCommandTooShort,

    /// The USB host sent a command that contains an unknown destination prefix.
    UsbCommandUnknownDestination,

    /// The USB host disconnected.
    UsbDisconnected,

    /// The USB host took too long to send a command.
    UsbTimeout,

    /// An unknown USB error (communicating with the USB host).
    UsbUnknown,
}

impl Error {
    /// Converts the error into a response that can be sent to the USB host.
    pub fn to_response(&self) -> &[u8] {
        match self {
            Error::UartResponseTooLong => "ERR:UART_RESPONSE_TOO_LONG\r\n",
            Error::UartTimeout => "ERR:UART_TIMEOUT\r\n",
            Error::UartUnknown => "ERR:UART_UNKNOWN\r\n",
            Error::UsbCommandMissingDestination => "ERR:USB_COMMAND_MISSING_DESTINATION\r\n",
            Error::UsbCommandTooLong => "ERR:USB_COMMAND_TOO_LONG\r\n",
            Error::UsbCommandTooShort => "ERR:USB_COMMAND_TOO_SHORT\r\n",
            Error::UsbCommandUnknownDestination => "ERR:USB_COMMAND_UNKNOWN_DESTINATION\r\n",
            Error::UsbDisconnected => "ERR:USB_DISCONNECTED\r\n",
            Error::UsbTimeout => "ERR:USB_TIMEOUT\r\n",
            Error::UsbUnknown => "ERR:USB_UNKNOWN\r\n",
        }
        .as_bytes()
    }
}
