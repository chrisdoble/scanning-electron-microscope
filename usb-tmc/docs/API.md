# usb-tmc

A small async Rust library for communicating with a single USBTMC (USB Test
and Measurement Class) instrument using bulk transfers, built on [`rusb`].

## Scope

This crate implements the subset of the USBTMC 1.00 specification needed to
exchange SCPI-style messages with an instrument over its bulk endpoints:

- Finding a device by USB vendor/product ID, opening it, and locating its
  USBTMC interface and bulk endpoints — once, at startup.
- Sending a message to the device (`DEV_DEP_MSG_OUT`).
- Requesting a response and reading it back in full
  (`REQUEST_DEV_DEP_MSG_IN` / `DEV_DEP_MSG_IN`), reassembling multi-transfer
  responses using the End-Of-Message (EOM) bit.

See [Out of scope](#out-of-scope) for what's deliberately not included.

The public API is `async`, built on `tokio`. The underlying `rusb` calls are
blocking; this crate runs them on `tokio::task::spawn_blocking` internally.

[`rusb`]: https://docs.rs/rusb

## `UsbTmcDevice`

```rust
/// A handle to a single USBTMC device, opened for bulk communication.
///
/// `UsbTmcDevice` is cheap to clone: clones share the same underlying USB
/// device handle and serialize their bulk transactions against each other,
/// so it can be cloned and moved into multiple tasks freely. The device's
/// USB interface is released once the last clone is dropped.
#[derive(Debug, Clone)]
pub struct UsbTmcDevice { /* private fields */ }
```

### Opening a device

```rust
impl UsbTmcDevice {
    /// Finds, opens, and prepares the USBTMC device with the given USB
    /// vendor and product ID for bulk communication.
    ///
    /// This searches all connected USB devices for one matching `vendor_id`
    /// and `product_id`, opens it, finds its USBTMC interface (an interface
    /// with class `0xFE` and subclass `0x03`) and that interface's bulk-IN
    /// and bulk-OUT endpoints, and claims it. All of this happens once;
    /// [`read`] and [`write`] reuse the result.
    ///
    /// `timeout` is the timeout applied to every bulk transfer performed by
    /// the returned device for its entire lifetime — there is no per-call
    /// override. If `timeout` is `None`, a default of 5 seconds is used.
    ///
    /// # Errors
    ///
    /// - [`Error::DeviceNotFound`] — no connected device matches `vendor_id`
    ///   and `product_id`.
    /// - [`Error::MultipleDevicesFound`] — more than one connected device
    ///   matches. This crate doesn't support disambiguating further; ensure
    ///   only one matching device is connected.
    /// - [`Error::UsbTmcInterfaceNotFound`] — the device has no interface
    ///   with the USBTMC class/subclass exposing exactly one bulk-IN and one
    ///   bulk-OUT endpoint.
    /// - [`Error::Usb`] — any underlying `rusb`/libusb call fails (opening
    ///   the device, claiming the interface, etc.).
    ///
    /// [`read`]: UsbTmcDevice::read
    /// [`write`]: UsbTmcDevice::write
    pub async fn open(
        vendor_id: u16,
        product_id: u16,
        timeout: Option<Duration>,
    ) -> Result<UsbTmcDevice>;
}
```

### Writing

```rust
impl UsbTmcDevice {
    /// Sends `data` to the device as a single USBTMC message
    /// (`DEV_DEP_MSG_OUT`).
    ///
    /// `data` is the raw message body. For SCPI instruments this is
    /// typically an ASCII command followed by a terminator such as `\n`
    /// (e.g. `b"*IDN?\n"`); this crate doesn't add, remove, or validate any
    /// terminator. If `data` doesn't fit in a single bulk-OUT transfer, it's
    /// automatically split across multiple transfers — the device sees one
    /// logical message regardless.
    ///
    /// Returns once the message has been fully written. This does not wait
    /// for or read a response; call [`read`] afterwards if the message is a
    /// query.
    ///
    /// # Errors
    ///
    /// [`Error::Usb`] if a bulk-OUT transfer fails or times out.
    ///
    /// [`read`]: UsbTmcDevice::read
    pub async fn write(&self, data: &[u8]) -> Result<()>;

    /// Sends `command` to the device, encoded as bytes.
    ///
    /// Equivalent to `self.write(command.as_bytes())`. `command` should
    /// include any terminator the instrument expects (typically `\n`).
    pub async fn write_str(&self, command: &str) -> Result<()>;
}
```

### Reading

```rust
impl UsbTmcDevice {
    /// Requests and reads back a single message from the device
    /// (`REQUEST_DEV_DEP_MSG_IN` / `DEV_DEP_MSG_IN`).
    ///
    /// Issues a request, then performs as many bulk-IN transfers as the
    /// device needs to send its response, concatenating their payloads. The
    /// device marks the final transfer with the End-Of-Message (EOM) flag;
    /// `read` keeps transferring until it sees that flag and returns the
    /// complete message.
    ///
    /// The request places no upper bound on the response size, so the
    /// returned buffer grows to fit whatever the device sends. Note this
    /// means the entire message is buffered in memory before `read`
    /// returns — relevant for instruments that can return very large
    /// responses (e.g. waveform data).
    ///
    /// The returned bytes are the message payload exactly as sent by the
    /// device, with no trimming or terminator handling — for SCPI responses
    /// this typically includes a trailing `\n`.
    ///
    /// # Errors
    ///
    /// - [`Error::Usb`] — a bulk transfer failed or timed out. A timeout
    ///   commonly means no response was pending (e.g. `read` was called
    ///   without a preceding query).
    /// - [`Error::Protocol`] — the device's response didn't conform to the
    ///   expected USBTMC framing (e.g. its `bTag` didn't match the
    ///   request). This means the host and device are out of sync; the
    ///   [`UsbTmcDevice`] should be dropped and reopened.
    pub async fn read(&self) -> Result<Vec<u8>>;

    /// Equivalent to [`read`](UsbTmcDevice::read), but decodes the response
    /// as UTF-8.
    ///
    /// # Errors
    ///
    /// In addition to the errors returned by [`read`](UsbTmcDevice::read),
    /// returns [`Error::Utf8`] if the response isn't valid UTF-8.
    pub async fn read_str(&self) -> Result<String>;

    /// Writes `command` and reads back the device's response, without
    /// allowing another transaction to be interleaved between the two.
    ///
    /// Equivalent to [`write_str`](UsbTmcDevice::write_str) followed by
    /// [`read`](UsbTmcDevice::read). `command` should include any
    /// terminator the instrument expects (typically `\n`).
    pub async fn query(&self, command: &str) -> Result<Vec<u8>>;

    /// Equivalent to [`query`](UsbTmcDevice::query), but decodes the
    /// response as UTF-8.
    ///
    /// # Errors
    ///
    /// In addition to the errors returned by [`query`](UsbTmcDevice::query),
    /// returns [`Error::Utf8`] if the response isn't valid UTF-8.
    pub async fn query_str(&self, command: &str) -> Result<String>;
}
```

### Errors

```rust
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
    #[error("{count} devices found with vendor ID {vendor_id:#06x} and product ID {product_id:#06x}, expected exactly 1")]
    MultipleDevicesFound { vendor_id: u16, product_id: u16, count: usize },

    /// The device has no USBTMC interface (class `0xFE`, subclass `0x03`)
    /// exposing exactly one bulk-IN and one bulk-OUT endpoint.
    #[error("no usable USBTMC interface found on device")]
    UsbTmcInterfaceNotFound,

    /// The device sent a response that didn't conform to the USBTMC bulk
    /// transfer framing (e.g. a mismatched `bTag`/`bTagInverse`, or an
    /// unexpected message type). The host and device are now out of sync;
    /// the [`UsbTmcDevice`] should be dropped and reopened.
    #[error("protocol error: {0}")]
    Protocol(String),

    /// An error from the underlying `rusb`/libusb library — e.g. while
    /// enumerating devices, opening a device, claiming an interface, or
    /// performing a bulk transfer (including timeouts).
    #[error(transparent)]
    Usb(#[from] rusb::Error),

    /// A response from [`read_str`](UsbTmcDevice::read_str) or
    /// [`query_str`](UsbTmcDevice::query_str) wasn't valid UTF-8.
    #[error("response is not valid UTF-8: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
}
```

## Usage example

```rust
use std::time::Duration;
use usb_tmc::UsbTmcDevice;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Rigol DHO800/900 series.
    let scope = UsbTmcDevice::open(0x1ab1, 0x044c, None).await?;

    let response = scope.query_str("*IDN?\n").await?;
    println!("{response}");

    Ok(())
}
```

## Out of scope

Deliberately not implemented in v1 (may be revisited later):

- **Control-endpoint requests** (`INITIATE_ABORT_BULK_OUT`,
  `CHECK_ABORT_BULK_OUT_STATUS`, `INITIATE_ABORT_BULK_IN`,
  `CHECK_ABORT_BULK_IN_STATUS`, `INITIATE_CLEAR`/`CHECK_CLEAR_STATUS`,
  `GET_CAPABILITIES`). Without these, the only way to recover from a stuck
  or desynchronized transfer is to drop and reopen the `UsbTmcDevice`.
- **The optional interrupt-IN endpoint** (used for asynchronous device
  notifications) is never read.
- **`TermChar`-based early termination** — `read` relies solely on the EOM
  bit; the `TermCharEnabled`/`TermChar` fields of `REQUEST_DEV_DEP_MSG_IN`
  are always left unset.
- **Disambiguating multiple matching devices** — `open` errors rather than
  picking one (e.g. by serial number or USB bus address).
- **Detaching/reattaching a kernel driver** — this crate targets macOS,
  where libusb's kernel-driver operations are unsupported (and unnecessary)
  anyway. `open` claims the interface directly and doesn't attempt to detach
  a kernel driver first.
