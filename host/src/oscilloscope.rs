use log::*;
use thiserror::Error;
use usb_tmc::UsbTmcDevice;

/// The USB product ID of the Rigol DHO-814 oscilloscope.
///
/// Shared by the whole DHO800/900 series.
const PRODUCT_ID: u16 = 0x044d;

/// The USB vendor ID of the Rigol DHO-814 oscilloscope.
const VENDOR_ID: u16 = 0x1ab1;

/// An error returned from the oscilloscope.
#[derive(Debug, Error)]
pub enum OscilloscopeError {
    /// The method hasn't been implemented yet.
    #[error("not implemented")]
    NotImplemented,

    /// An error encountered while communicating with the scope over USBTMC.
    #[error("usb-tmc error: {0}")]
    UsbTmc(#[from] usb_tmc::Error),
}

/// Interacts with the Rigol DHO-814 oscilloscope.
///
/// Only some commands and queries are implemented. See the scope's programming
/// guide for a list of all supported commands and queries.
#[derive(Debug)]
pub struct Oscilloscope {
    /// The scope itself.
    ///
    /// `UsbTmcDevice` already serialises individual transactions, but a
    /// measurement is a sequence of them — add the measurement item, wait for
    /// the scope to sample, then query it — which must not interleave with
    /// another caller's, so it needs a lock of its own.
    // TODO: remove this once the methods below read the device.
    #[allow(dead_code)]
    device: tokio::sync::Mutex<UsbTmcDevice>,
}

impl Oscilloscope {
    /// Found by its USB vendor and product IDs.
    pub async fn new() -> Result<Self, OscilloscopeError> {
        let device = UsbTmcDevice::open(VENDOR_ID, PRODUCT_ID, None)
            .await
            .inspect_err(|e| error!("failed to open the oscilloscope: {}", e))?;

        Ok(Self {
            device: tokio::sync::Mutex::new(device),
        })
    }

    /// Measures the mean voltage on `channel` in volts.
    pub async fn measure_voltage(&self, _channel: u8) -> Result<f64, OscilloscopeError> {
        // TODO: set the scope running, add the measurement item by sending
        // ":MEASure:ITEM? VAVG,CHANnel<channel>", give it time to sample, then
        // query the same command and parse the response. See
        // `usb-tmc/src/bin/oscilloscope.rs` for the sequence.
        error!("oscilloscope measure_voltage isn't implemented");
        Err(OscilloscopeError::NotImplemented)
    }
}
