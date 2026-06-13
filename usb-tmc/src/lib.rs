//! A small async library for communicating with a single USBTMC (USB Test
//! and Measurement Class) instrument over its bulk endpoints, built on
//! [`rusb`]. See `docs/API.md` for the full API design.

mod error;
mod protocol;

use rusb::{Direction, GlobalContext, TransferType};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use error::{Error, Result};

/// The timeout used by [`UsbTmcDevice::open`] when `timeout` is `None`.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// The size of the buffer used to read bulk-IN transfers.
///
/// Must be a multiple of 4 — see the comment in [`read_response`] for why.
const READ_CHUNK_LEN: usize = 16 * 1024;

/// The maximum number of bytes written to the bulk-OUT endpoint in a single
/// `rusb` call.
const WRITE_CHUNK_LEN: usize = 16 * 1024;

/// A handle to a single USBTMC device, opened for bulk communication.
///
/// `UsbTmcDevice` is cheap to clone: clones share the same underlying USB
/// device handle and serialize their bulk transactions against each other,
/// so it can be cloned and moved into multiple tasks freely. The device's
/// USB interface is released once the last clone is dropped.
#[derive(Debug, Clone)]
pub struct UsbTmcDevice {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    handle: rusb::DeviceHandle<GlobalContext>,
    interface_number: u8,
    bulk_in_endpoint: u8,
    bulk_out_endpoint: u8,
    timeout: Duration,
    b_tags: protocol::BTagSequence,
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inner")
            .field("interface_number", &self.interface_number)
            .field("bulk_in_endpoint", &self.bulk_in_endpoint)
            .field("bulk_out_endpoint", &self.bulk_out_endpoint)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Err(e) = self.handle.release_interface(self.interface_number) {
            log::error!("failed to release USBTMC interface: {e}");
        }
    }
}

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
    ) -> Result<UsbTmcDevice> {
        let timeout = timeout.unwrap_or(DEFAULT_TIMEOUT);
        tokio::task::spawn_blocking(move || open_blocking(vendor_id, product_id, timeout))
            .await
            .expect("usb-tmc worker thread panicked")
    }

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
    pub async fn write(&self, data: &[u8]) -> Result<()> {
        let inner = Arc::clone(&self.inner);
        let data = data.to_vec();
        tokio::task::spawn_blocking(move || {
            write_message(&mut inner.lock().expect("usb-tmc mutex poisoned"), &data)
        })
        .await
        .expect("usb-tmc worker thread panicked")
    }

    /// Sends `command` to the device, encoded as bytes.
    ///
    /// Equivalent to `self.write(command.as_bytes())`. `command` should
    /// include any terminator the instrument expects (typically `\n`).
    pub async fn write_str(&self, command: &str) -> Result<()> {
        self.write(command.as_bytes()).await
    }

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
    pub async fn read(&self) -> Result<Vec<u8>> {
        let inner = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            read_message(&mut inner.lock().expect("usb-tmc mutex poisoned"))
        })
        .await
        .expect("usb-tmc worker thread panicked")
    }

    /// Equivalent to [`read`](UsbTmcDevice::read), but decodes the response
    /// as UTF-8.
    ///
    /// # Errors
    ///
    /// In addition to the errors returned by [`read`](UsbTmcDevice::read),
    /// returns [`Error::Utf8`] if the response isn't valid UTF-8.
    pub async fn read_str(&self) -> Result<String> {
        Ok(String::from_utf8(self.read().await?)?)
    }

    /// Writes `command` and reads back the device's response, without
    /// allowing another transaction to be interleaved between the two.
    ///
    /// Equivalent to [`write_str`](UsbTmcDevice::write_str) followed by
    /// [`read`](UsbTmcDevice::read). `command` should include any
    /// terminator the instrument expects (typically `\n`).
    pub async fn query(&self, command: &str) -> Result<Vec<u8>> {
        let inner = Arc::clone(&self.inner);
        let command = command.as_bytes().to_vec();
        tokio::task::spawn_blocking(move || {
            let mut inner = inner.lock().expect("usb-tmc mutex poisoned");
            write_message(&mut inner, &command)?;
            read_message(&mut inner)
        })
        .await
        .expect("usb-tmc worker thread panicked")
    }

    /// Equivalent to [`query`](UsbTmcDevice::query), but decodes the
    /// response as UTF-8.
    ///
    /// # Errors
    ///
    /// In addition to the errors returned by [`query`](UsbTmcDevice::query),
    /// returns [`Error::Utf8`] if the response isn't valid UTF-8.
    pub async fn query_str(&self, command: &str) -> Result<String> {
        Ok(String::from_utf8(self.query(command).await?)?)
    }
}

fn open_blocking(vendor_id: u16, product_id: u16, timeout: Duration) -> Result<UsbTmcDevice> {
    let device = find_device(vendor_id, product_id)?;
    let (interface_number, bulk_in_endpoint, bulk_out_endpoint) = find_usbtmc_interface(&device)?;

    let handle = device.open()?;
    handle.claim_interface(interface_number)?;

    Ok(UsbTmcDevice {
        inner: Arc::new(Mutex::new(Inner {
            handle,
            interface_number,
            bulk_in_endpoint,
            bulk_out_endpoint,
            timeout,
            b_tags: protocol::BTagSequence::default(),
        })),
    })
}

/// Finds the single connected USB device matching `vendor_id` and
/// `product_id`.
fn find_device(vendor_id: u16, product_id: u16) -> Result<rusb::Device<GlobalContext>> {
    let matching: Vec<_> = rusb::devices()?
        .iter()
        .filter(|device| {
            device
                .device_descriptor()
                .map(|descriptor| {
                    descriptor.vendor_id() == vendor_id && descriptor.product_id() == product_id
                })
                .unwrap_or(false)
        })
        .collect();

    match matching.len() {
        0 => Err(Error::DeviceNotFound {
            vendor_id,
            product_id,
        }),
        1 => Ok(matching.into_iter().next().unwrap()),
        count => Err(Error::MultipleDevicesFound {
            vendor_id,
            product_id,
            count,
        }),
    }
}

/// Finds `device`'s USBTMC interface and that interface's bulk-IN and
/// bulk-OUT endpoints, returning `(interface_number, bulk_in_endpoint,
/// bulk_out_endpoint)`.
fn find_usbtmc_interface(device: &rusb::Device<GlobalContext>) -> Result<(u8, u8, u8)> {
    let config = device.active_config_descriptor()?;

    for interface in config.interfaces() {
        for descriptor in interface.descriptors() {
            if descriptor.class_code() != protocol::USBTMC_INTERFACE_CLASS
                || descriptor.sub_class_code() != protocol::USBTMC_INTERFACE_SUBCLASS
            {
                continue;
            }

            let bulk_in = descriptor.endpoint_descriptors().find(|e| {
                e.direction() == Direction::In && e.transfer_type() == TransferType::Bulk
            });
            let bulk_out = descriptor.endpoint_descriptors().find(|e| {
                e.direction() == Direction::Out && e.transfer_type() == TransferType::Bulk
            });

            if let (Some(bulk_in), Some(bulk_out)) = (bulk_in, bulk_out) {
                return Ok((
                    descriptor.interface_number(),
                    bulk_in.address(),
                    bulk_out.address(),
                ));
            }
        }
    }

    Err(Error::UsbTmcInterfaceNotFound)
}

/// Sends `data` as a single `DEV_DEP_MSG_OUT` message.
fn write_message(inner: &mut Inner, data: &[u8]) -> Result<()> {
    let b_tag = inner.b_tags.next();
    let header = protocol::encode_dev_dep_msg_out(b_tag, data.len() as u32, true);
    let padding = protocol::padding_len(data.len());

    let mut message = Vec::with_capacity(protocol::HEADER_LEN + data.len() + padding);
    message.extend_from_slice(&header);
    message.extend_from_slice(data);
    message.resize(message.len() + padding, 0);

    write_bulk_all(
        &inner.handle,
        inner.bulk_out_endpoint,
        &message,
        inner.timeout,
    )
}

/// Requests and reads back a single message, looping over
/// `REQUEST_DEV_DEP_MSG_IN`/`DEV_DEP_MSG_IN` exchanges until the device sets
/// EOM.
fn read_message(inner: &mut Inner) -> Result<Vec<u8>> {
    let mut message = Vec::new();

    loop {
        let b_tag = inner.b_tags.next();
        // The Rigol DP932E power supply gets confused when the transfer size is
        // set to u32::MAX. It behaves as expected when using pyvisa which uses
        // a transfer size of 20 * 1024. Use that here so everything works.
        let request = protocol::encode_request_dev_dep_msg_in(b_tag, 20 * 1024);
        write_bulk_all(
            &inner.handle,
            inner.bulk_out_endpoint,
            &request,
            inner.timeout,
        )?;

        let (payload, eom) = read_response(inner, b_tag)?;
        message.extend(payload);

        if eom {
            return Ok(message);
        }
    }
}

/// Reads a single `DEV_DEP_MSG_IN` response (header and payload) for the
/// request with the given `b_tag`, returning its payload and EOM flag.
fn read_response(inner: &Inner, b_tag: u8) -> Result<(Vec<u8>, bool)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; READ_CHUNK_LEN];

    while buf.len() < protocol::HEADER_LEN {
        read_bulk_into(inner, &mut chunk, &mut buf)?;
    }

    let header: [u8; protocol::HEADER_LEN] = buf[..protocol::HEADER_LEN].try_into().unwrap();
    let response = protocol::decode_dev_dep_msg_in(&header, b_tag)?;

    let transfer_size = response.transfer_size as usize;

    // Per the spec this transfer should be padded with up to 3 bytes to a
    // multiple of 4, but not all devices send that padding, so don't wait
    // for it. This is safe as long as READ_CHUNK_LEN is a multiple of 4: a
    // `read_bulk` call only returns early because `chunk` filled up (rather
    // than because the device ended the transfer) when `buf.len()` is a
    // multiple of READ_CHUNK_LEN, and since HEADER_LEN is also a multiple of
    // 4, that implies `transfer_size` is too — i.e. there's no padding left
    // to strand in the device's send buffer in that case.
    let total_len = protocol::HEADER_LEN + transfer_size;

    while buf.len() < total_len {
        let n = read_bulk_into(inner, &mut chunk, &mut buf)?;
        if n < chunk.len() && buf.len() < total_len {
            return Err(Error::Protocol(format!(
                "device's DEV_DEP_MSG_IN transfer ended after {} bytes, expected at least {total_len}",
                buf.len()
            )));
        }
    }

    let payload = buf[protocol::HEADER_LEN..protocol::HEADER_LEN + transfer_size].to_vec();
    Ok((payload, response.eom))
}

/// Reads one bulk-IN transfer into `chunk`, appends it to `buf`, and returns
/// the number of bytes read.
fn read_bulk_into(inner: &Inner, chunk: &mut [u8], buf: &mut Vec<u8>) -> Result<usize> {
    let n = inner
        .handle
        .read_bulk(inner.bulk_in_endpoint, chunk, inner.timeout)?;
    if n == 0 {
        return Err(Error::Protocol(
            "device ended bulk-IN transfer unexpectedly".to_string(),
        ));
    }
    buf.extend_from_slice(&chunk[..n]);
    Ok(n)
}

/// Writes all of `data` to `endpoint`, looping as necessary.
fn write_bulk_all(
    handle: &rusb::DeviceHandle<GlobalContext>,
    endpoint: u8,
    mut data: &[u8],
    timeout: Duration,
) -> Result<()> {
    while !data.is_empty() {
        let chunk_len = data.len().min(WRITE_CHUNK_LEN);
        let written = handle.write_bulk(endpoint, &data[..chunk_len], timeout)?;
        data = &data[written..];
    }
    Ok(())
}
