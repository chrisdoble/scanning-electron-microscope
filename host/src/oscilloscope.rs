use std::{
    fmt::Display,
    num::ParseFloatError,
    time::{Duration, Instant},
};

use log::*;
use thiserror::Error;
use usb_tmc::UsbTmcDevice;

/// The magnitude of the voltage the oscilloscope reports when a measurement is
/// invalid, e.g. because the signal is off screen.
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
    /// A single acquisition didn't complete in time.
    #[error("the acquisition didn't complete within {0:?}")]
    AcquisitionTimeout(Duration),

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
}

/// The voltages on channel 1 over a single 20 ms acquisition, in volts.
///
/// Each is `None` if the scope still reported it as invalid after a few
/// attempts, which means the signal went off screen: the voltage is clipped.
#[derive(Clone, Copy, Debug)]
pub struct Voltages {
    pub average: Option<f64>,
    pub maximum: Option<f64>,
    pub minimum: Option<f64>,
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
            state: tokio::sync::Mutex::new(OscilloscopeState { device }),
        })
    }

    /// Measures the average, maximum and minimum voltages on channel 1 over a
    /// single 20 ms acquisition.
    ///
    /// 20 ms is one cycle of Australian mains voltage at 50 Hz, so the average
    /// cancels mains noise.
    ///
    /// A single acquisition, rather than querying a running scope, guarantees
    /// that all three values come from the same acquisition, that the
    /// acquisition started after this was called, and that consecutive calls
    /// are independent however fast the scope's measurement engine updates.
    /// The scope is left stopped afterwards.
    pub async fn get_voltages(&self) -> Result<Voltages, OscilloscopeError> {
        // How long to wait for the acquisition, and how often to check on it.
        // An acquisition has been seen to take ~360 ms, most of it after the
        // force, and each status query takes ~30 ms on its own.
        const ACQUISITION_TIMEOUT: Duration = Duration::from_secs(2);
        const STATUS_INTERVAL: Duration = Duration::from_millis(50);

        let state = self.state.lock().await;

        // See `set_vertical_scale` for why this is a clone of the handle.
        let device = state.device.clone();

        device.write_str(":SINGle").await?;

        let start = Instant::now();
        let mut forced = false;
        loop {
            match device.query_str(":TRIGger:STATus?").await?.trim() {
                // The acquisition is complete. Noise crossing the trigger level
                // can trigger it before it's forced, which is fine: it's still
                // an acquisition that began after the arm.
                "STOP" => break,

                // Armed and waiting. A DC signal may never cross the trigger
                // level, so force it. Waiting for this rather than forcing
                // straight away means the force can't arrive before the scope
                // has armed, when it might be ignored.
                "WAIT" if !forced => {
                    device.write_str(":TFORce").await?;
                    forced = true;
                }

                _ => {}
            }

            if start.elapsed() >= ACQUISITION_TIMEOUT {
                error!(
                    "the acquisition didn't complete within {:?}",
                    ACQUISITION_TIMEOUT
                );
                return Err(OscilloscopeError::AcquisitionTimeout(ACQUISITION_TIMEOUT));
            }

            tokio::time::sleep(STATUS_INTERVAL).await;
        }

        // The scope is stopped, so all three come from the acquisition above.
        Ok(Voltages {
            average: get_item(device.clone(), "VAVG").await?,
            maximum: get_item(device.clone(), "VMAX").await?,
            minimum: get_item(device, "VMIN").await?,
        })
    }

    /// Resets the oscilloscope, making it ready for use.
    ///
    /// Everything a measurement depends on is set explicitly rather than left
    /// to however the scope was last used.
    pub async fn reset(&self) -> Result<(), OscilloscopeError> {
        let state = self.state.lock().await;
        let device = &state.device;

        // Stop the scope
        device.write_str(":STOP").await?;

        // Clear all measurements
        device.write_str(":MEASure:CLEar").await?;

        // Measure over the main time base rather than a zoom window
        device.write_str(":MEASure:AREA MAIN").await?;

        // Measure the average, maximum and minimum voltages on channel 1. The
        // average alone can't detect clipping: it stays defined while the
        // peaks are clipped. This is the command form rather than the query
        // form, so there's no response to read and nothing left in the scope's
        // output queue for the next query to pick up by mistake.
        for item in ["VAVG", "VMAX", "VMIN"] {
            device
                .write_str(format!(":MEASure:ITEM {},CHANnel1", item).as_str())
                .await?;
        }

        // Acquire normally. Averaging would make consecutive samples dependent,
        // and peak detection exaggerates noise.
        device.write_str(":ACQuire:TYPE NORMal").await?;

        // Acquire even when nothing triggers, so the display shows a live trace
        // until the first `get_voltages` switches the sweep to single. A DC
        // signal never crosses the trigger level.
        device.write_str(":TRIGger:SWEep AUTO").await?;

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

        // DC coupling, since the signal is DC
        device.write_str(":CHANnel1:COUPling DC").await?;

        // Limit the bandwidth to 20 MHz. The signal is DC, so everything above
        // it is noise, which would inflate the maximum and minimum.
        device.write_str(":CHANnel1:BWLimit 20M").await?;

        // A 1× probe. This goes before the vertical scale, because the probe
        // ratio changes the scale's valid range.
        device.write_str(":CHANnel1:PROBe 1").await?;

        // Set the offset of channel 1 to 0 V
        device.write_str(":CHANnel1:OFFSet 0").await?;

        // Set the vertical scale of channel 1 to 10 mV/div
        device.write_str(":CHANnel1:SCALe 0.01").await?;

        // Run the scope
        device.write_str(":RUN").await?;

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

        // Clone the handle so `poll`'s futures own it rather than borrowing the
        // guard (see `poll`). Clones of a `UsbTmcDevice` share the same device
        // and serialise against each other, and the guard is still held, so the
        // transaction is unchanged.
        let device = state.device.clone();

        // The scope takes a moment to apply the new scale, so read it back
        // until it's the value we set.
        poll(move || {
            let device = device.clone();
            async move {
                let f: f64 = device.query_str(":CHANnel1:SCALe?").await?.trim().parse()?;

                Ok(if f == scale {
                    Ok(())
                } else {
                    Err(OscilloscopeError::Unknown(format!(
                        "vertical scale is {}, expected {}",
                        f, scale
                    )))
                })
            }
        })
        .await?
    }
}

/// Queries measurement `item` (e.g. `VAVG`) on channel 1, or `None` if the
/// scope still reports it as invalid after a few attempts.
///
/// A newly added measurement reads as invalid until it has run for long
/// enough to have a reading, so that's worth retrying. It reads the same when
/// the signal really is off screen, which is what `None` means once we give
/// up.
///
/// Takes its own handle, and `item` as `'static`, so the futures `poll` runs
/// borrow nothing (see `poll`).
async fn get_item(
    device: UsbTmcDevice,
    item: &'static str,
) -> Result<Option<f64>, OscilloscopeError> {
    let query = format!(":MEASure:ITEM? {},CHANnel1", item);

    Ok(poll(move || {
        let device = device.clone();
        let query = query.clone();
        async move {
            // e.g. -4.2016E-04
            let voltage: f64 = device.query_str(&query).await?.trim().parse()?;

            Ok(if voltage.abs() == CLIPPED_VOLTAGE {
                Err(format!("{} is invalid", item))
            } else {
                Ok(voltage)
            })
        }
    })
    .await?
    .ok())
}

/// Calls `f` until it succeeds, a few times, with a short wait between
/// attempts.
///
/// `f` returns `Ok(Ok(value))` when it's done, `Ok(Err(e))` when the scope
/// hasn't caught up yet and the call is worth repeating, and `Err(e)` for a
/// failure no retry can fix — so a `?` inside `f` is the fatal path.
///
/// The last retryable error is returned as `Ok(Err(e))` once the attempts run
/// out, so the caller decides what running out means: `get_item` treats it as
/// a clipped voltage, and `set_vertical_scale` as a failure.
///
/// `f` is a plain closure returning an owned future rather than an async
/// closure, whose futures borrow the closure. Callers hold the state's lock
/// while this runs, and a future that holds a borrow alongside a borrowing
/// async closure can't be proven `Send` for every lifetime, which breaks
/// callers that box it — an `#[async_trait]` implementation, say.
async fn poll<T, E: Display, F: Future<Output = Result<Result<T, E>, OscilloscopeError>>>(
    mut f: impl FnMut() -> F,
) -> Result<Result<T, E>, OscilloscopeError> {
    // The number of times `f` is attempted before we give up, and how long we
    // wait between attempts.
    const ATTEMPTS: u32 = 3;
    const INTERVAL: Duration = Duration::from_millis(100);

    let mut attempt = 1;

    loop {
        match f().await? {
            Ok(value) => return Ok(Ok(value)),

            // Compare with `>=` rather than `==` so this can't loop forever if
            // `ATTEMPTS` is ever set to 0.
            Err(e) if attempt >= ATTEMPTS => {
                warn!("giving up after {} attempts: {}", ATTEMPTS, e);
                return Ok(Err(e));
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
