use std::{num::ParseFloatError, time::Duration};

use log::*;
use thiserror::Error;
use usb_tmc::UsbTmcDevice;

/// The magnitude of the voltage the oscilloscope reports when a measurement is
/// off screen.
///
/// It reports the same value for a measurement it has been given but hasn't yet
/// run for long enough to have a reading for.
const CLIPPED_VOLTAGE: f64 = 9.9e37;

/// The USB product ID of the Rigol DHO-814 oscilloscope.
///
/// Shared by the whole DHO800/900 series.
const PRODUCT_ID: u16 = 0x044d;

/// The USB vendor ID of the Rigol DHO-814 oscilloscope.
const VENDOR_ID: u16 = 0x1ab1;

/// An error returned from the oscilloscope.
#[derive(Debug, Error)]
pub enum OscilloscopeError {
    /// A voltage was larger than the oscilloscope's vertical range.
    #[error("clipped voltage")]
    ClippedVoltage,

    /// An argument was invalid.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("invalid voltage: {0}")]
    InvalidVoltage(#[from] ParseFloatError),

    /// The method hasn't been implemented yet.
    #[error("not implemented")]
    NotImplemented,

    /// A catch all error for anything else that might happen.
    #[error("unknown error: {0}")]
    Unknown(String),

    /// An error encountered while communicating with the scope over USBTMC.
    #[error("usb-tmc error: {0}")]
    UsbTmc(#[from] usb_tmc::Error),
}

#[derive(Debug)]
struct OscilloscopeState {
    /// A handle to the scope.
    device: UsbTmcDevice,

    /// Whether the scope is currently measuring the average voltage.
    ///
    /// We cache this locally so we don't have to query the scope's state on
    /// each call to `measure_voltage`.
    is_measuring_voltage: bool,
}

/// Interacts with the Rigol DHO-814 oscilloscope on channel 1.
///
/// Only some commands and queries are implemented. See the scope's programming
/// guide for a list of all supported commands and queries.
#[derive(Debug)]
pub struct Oscilloscope {
    state: tokio::sync::Mutex<OscilloscopeState>,
}

impl Oscilloscope {
    /// Found by its USB vendor and product IDs.
    pub async fn new() -> Result<Self, OscilloscopeError> {
        let device = UsbTmcDevice::open(VENDOR_ID, PRODUCT_ID, None)
            .await
            .inspect_err(|e| error!("failed to open the oscilloscope: {}", e))?;

        Ok(Self {
            state: tokio::sync::Mutex::new(OscilloscopeState {
                device,
                is_measuring_voltage: false,
            }),
        })
    }

    /// Measures the mean voltage on channel 1 over 20 ms in volts.
    ///
    /// 20 ms was chosen to cancel noise from Australian mains voltage at 50 Hz.
    pub async fn get_voltage(&self) -> Result<f64, OscilloscopeError> {
        let mut state = self.state.lock().await;

        // Add the measurement if it isn't already running. This is the command
        // form rather than the query form, so there's no response to read and
        // nothing left in the scope's output queue for the next query to pick
        // up by mistake.
        if !state.is_measuring_voltage {
            state
                .device
                .write_str(":MEASure:ITEM VAVG,CHANnel1")
                .await?;
            state.is_measuring_voltage = true;
        }

        // Clone the handle for the closure rather than letting it borrow the
        // guard. A future that captures a `&MutexGuard` can't be proven `Send`
        // for every lifetime, which breaks callers that box it — an
        // `#[async_trait]` implementation, say. Clones of a `UsbTmcDevice` share
        // the same device and serialise against each other, and the guard is
        // still held, so the transaction is unchanged.
        let device = state.device.clone();

        poll(async move || {
            // e.g. -4.2016E-04
            let voltage: f64 = device
                .query_str(":MEASure:ITEM? VAVG,CHANnel1")
                .await?
                .trim()
                .parse()?;

            // The scope reports the clipped voltage until the measurement has
            // run for long enough to have a reading, so this is worth
            // retrying. It reports the same value when the voltage really is
            // off screen, which is what the caller sees once we give up.
            if voltage.abs() == CLIPPED_VOLTAGE {
                return Ok(Err(OscilloscopeError::ClippedVoltage));
            }

            Ok(Ok(voltage))
        })
        .await
    }

    /// Resets the oscilloscope, making it ready for use.
    pub async fn reset(&self) -> Result<(), OscilloscopeError> {
        let mut state = self.state.lock().await;
        let device = &state.device;

        // Stop the scope
        device.write_str(":STOP").await?;

        // Clear all measurements
        device.write_str(":MEASure:CLEar").await?;

        // Clear the screen
        device.write_str(":CLEar").await?;

        // Turn off all channels
        for i in 1..=4 {
            device
                .write_str(format!(":CHANnel{}:DISPLAY OFF", i).as_str())
                .await?;
        }

        // Set the horizontal scale to 2 ms/div
        device.write_str(":TIMebase:SCALe 0.002").await?;

        // Show channel 1
        device.write_str(":CHANnel1:DISPLAY ON").await?;

        // Set the offset of channel 1 to 0 V
        device.write_str(":CHANnel1:OFFSet 0").await?;

        // Set the vertical scale of channel 1 to 10 mV/div
        device.write_str(":CHANnel1:SCALe 0.01").await?;

        // Run the scope
        device.write_str(":RUN").await?;

        state.is_measuring_voltage = false;
        Ok(())
    }

    /// Sets the vertical scale.
    ///
    /// Returns `Err(OscilloscopeError::InvalidArgument)` if `scale` isn't a
    /// finite number > 0.
    pub async fn set_vertical_scale(&self, scale: f64) -> Result<(), OscilloscopeError> {
        if scale.is_infinite() || scale.is_nan() || scale <= 0. {
            return Err(OscilloscopeError::InvalidArgument(format!(
                "vertical scale must be a finite number > 0: {}",
                scale
            )));
        }

        let state = self.state.lock().await;

        // Set the vertical scale.
        state
            .device
            .write_str(format!(":CHANnel1:SCALe {}", scale).as_str())
            .await?;

        // See `get_voltage` for why the closure gets a clone of the handle
        // rather than borrowing the guard.
        let device = state.device.clone();

        // The scope takes a moment to apply the new scale, so read it back
        // until it's the value we set.
        poll(async move || {
            let f: f64 = device.query_str(":CHANnel1:SCALe?").await?.trim().parse()?;

            Ok(if f == scale {
                Ok(())
            } else {
                Err(OscilloscopeError::Unknown(format!(
                    "vertical scale is {}, expected {}",
                    f, scale
                )))
            })
        })
        .await
    }
}

/// Calls `f` until it succeeds, a few times, with a short wait between
/// attempts.
///
/// `f` returns `Ok(Ok(value))` when it's done, `Ok(Err(e))` when the scope
/// hasn't caught up yet and the call is worth repeating, and `Err(e)` for a
/// failure no retry can fix — so a `?` inside `f` is the fatal path.
///
/// A retryable error is returned as it is once the attempts run out, which is
/// what lets `get_voltage` report `ClippedVoltage` when the voltage really is
/// off screen.
async fn poll<T>(
    mut f: impl AsyncFnMut() -> Result<Result<T, OscilloscopeError>, OscilloscopeError>,
) -> Result<T, OscilloscopeError> {
    // The number of times `f` is attempted before we give up, and how long we
    // wait between attempts.
    const ATTEMPTS: u32 = 3;
    const INTERVAL: Duration = Duration::from_millis(100);

    let mut attempt = 1;

    loop {
        match f().await? {
            Ok(value) => return Ok(value),

            // Compare with `>=` rather than `==` so this can't loop forever if
            // `ATTEMPTS` is ever set to 0.
            Err(e) if attempt >= ATTEMPTS => {
                error!("giving up after {} attempts: {}", ATTEMPTS, e);
                return Err(e);
            }

            Err(e) => {
                warn!(
                    "attempt {} of {} failed, retrying: {}",
                    attempt, ATTEMPTS, e
                );
                attempt += 1;
                tokio::time::sleep(INTERVAL).await;
            }
        }
    }
}
