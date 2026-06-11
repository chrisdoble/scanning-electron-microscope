use thiserror::Error;

/// The result type returned by fallible operations in this crate.
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    /// No connected USB device matched the requested vendor and product ID.
    #[error("no device found with vendor ID {vendor_id:#06x} and product ID {product_id:#06x}")]
    DeviceNotFound { vendor_id: u16, product_id: u16 },

    /// More than one connected USB device matched the requested vendor and
    /// product ID. This crate doesn't support disambiguating between
    /// multiple matches — ensure only one is connected.
    #[error(
        "{count} devices found with vendor ID {vendor_id:#06x} and product ID {product_id:#06x}, expected exactly 1"
    )]
    MultipleDevicesFound {
        vendor_id: u16,
        product_id: u16,
        count: usize,
    },

    /// The device has no USBTMC interface (class `0xFE`, subclass `0x03`)
    /// exposing exactly one bulk-IN and one bulk-OUT endpoint.
    #[error("no usable USBTMC interface found on device")]
    UsbTmcInterfaceNotFound,

    /// The device sent a response that didn't conform to the USBTMC bulk
    /// transfer framing (e.g. a mismatched `bTag`/`bTagInverse`, or an
    /// unexpected message type). The host and device are now out of sync;
    /// the [`UsbTmcDevice`](crate::UsbTmcDevice) should be dropped and
    /// reopened.
    #[error("protocol error: {0}")]
    Protocol(String),

    /// An error from the underlying `rusb`/libusb library — e.g. while
    /// enumerating devices, opening a device, claiming an interface, or
    /// performing a bulk transfer (including timeouts).
    #[error(transparent)]
    Usb(#[from] rusb::Error),

    /// A response from
    /// [`read_str`](crate::UsbTmcDevice::read_str)/[`query_str`](crate::UsbTmcDevice::query_str)
    /// wasn't valid UTF-8.
    #[error("response is not valid UTF-8: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
}
