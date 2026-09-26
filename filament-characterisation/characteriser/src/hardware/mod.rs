//! Everything the application can talk to, behind one trait per subsystem.

mod mock;
mod real;

pub use mock::{MockFilamentSystem, MockVacuumSystem};
pub use real::{RealFilamentSystem, RealVacuumSystem};

use async_trait::async_trait;
use host::{
    adc::{AdcError, Pressure},
    oscilloscope::OscilloscopeError,
    power_supply::{Polarity, PowerSupplyError},
    tmp::TmpError,
};
use log::*;
use std::{sync::Arc, time::Duration};
use thiserror::Error;
use tokio::{sync::watch, time::MissedTickBehavior};

/// How many polls in a row may fail before the hardware is given up on.
///
/// A serial timeout mid-run shouldn't end a forty-minute pump-down, so a single
/// failure is only logged and retried. But persistent failure means the rig is
/// no longer under control, and the filament shouldn't stay powered.
const POLL_FAILURE_TOLERANCE: u32 = 3;

/// How often the hardware is polled.
///
/// Once a second is plenty for a display, and the vacuum side alone is five
/// serial round trips, each of which the ADC can take ~300 ms to answer.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// An error encountered while reading or driving the hardware.
#[derive(Debug, Error)]
pub enum HardwareError {
    #[error("adc error: {0}")]
    Adc(#[from] AdcError),

    /// The serial link to the controller couldn't be opened.
    #[error("couldn't open the controller at {path}: {source}")]
    Controller {
        path: String,
        #[source]
        source: serialport::Error,
    },

    #[error("oscilloscope error: {0}")]
    Oscilloscope(#[from] OscilloscopeError),

    #[error("power supply error: {0}")]
    PowerSupply(#[from] PowerSupplyError),

    #[error("tmp error: {0}")]
    Tmp(#[from] TmpError),

    // Custom errors
    #[error("{0}")]
    Other(String),
}

/// A snapshot of the filament system, taken by the hardware poll task.
#[derive(Clone, Copy, Debug)]
pub struct FilamentSnapshot {
    /// The voltage across the filament in volts, from four-terminal sensing.
    pub filament_voltage: f64,

    /// The current flowing through the filament in amperes.
    pub heating_current: f64,

    /// If the power supply output is enabled.
    pub output_enabled: bool,

    /// How the filament is connected to the supply.
    pub polarity: Polarity,
}

/// A snapshot of the vacuum system, taken by the hardware poll task.
///
/// `Copy` because every field is, and snapshots are passed around by value.
/// Drop it if a non-`Copy` field is ever added. Deliberately not `Default`:
/// there is no such thing as a default reading, and the absence of readings is
/// modelled by the absence of a snapshot.
#[derive(Clone, Copy, Debug)]
pub struct VacuumSnapshot {
    /// The chamber pressure.
    pub pressure: Pressure,

    /// The current draw of the TMP in amperes.
    pub tmp_current: f32,

    /// The current rotation speed of the TMP in hertz.
    pub tmp_current_rotation_speed: u16,

    /// If the TMP is running.
    pub tmp_running: bool,

    /// The target rotation speed of the TMP in hertz.
    pub tmp_target_rotation_speed: u16,
}

/// Both snapshots, published together so the UI and the procedure always see a
/// consistent pair.
#[derive(Clone, Copy, Debug)]
pub struct Snapshots {
    pub filament: FilamentSnapshot,
    pub vacuum: VacuumSnapshot,
}

/// The filament system: the power supply, its polarity relays, and the
/// oscilloscope measuring the voltage across the filament.
#[async_trait]
pub trait FilamentSystem: std::fmt::Debug + Send + Sync {
    /// Puts the filament system into a safe state.
    ///
    /// Called on every exit path: normal completion, procedure error, user quit,
    /// a signal, and a procedure panic. Must be idempotent and must not return
    /// early on the first failure — every action is attempted and failures are
    /// logged.
    ///
    /// In order: zero the current, disable the output, then
    /// `set_polarity(Polarity::Nil)` to de-energise both relays.
    ///
    /// Returns the first failure, so the caller can warn that the filament
    /// system may still be powered.
    async fn enter_safe_state(&self) -> Result<(), HardwareError>;

    /// Measures the voltage across the filament in volts.
    ///
    /// This is the sense pair of the four-terminal measurement, so it excludes
    /// the drop across the supply leads and the feedthroughs.
    async fn get_filament_voltage(&self) -> Result<f64, HardwareError>;

    /// Measures the current through the filament in amperes.
    async fn get_heating_current(&self) -> Result<f64, HardwareError>;

    /// Sets the channel's current limit in amperes.
    ///
    /// Must be rejected above `MAXIMUM_HEATING_CURRENT_AMPS`.
    async fn set_heating_current(&self, current: f64) -> Result<(), HardwareError>;

    /// Sets the channel's voltage limit in volts.
    ///
    /// The supply runs in whichever mode its limits make it: with the voltage
    /// limit set high, the current limit is what binds and the channel runs in
    /// constant current. Called once at the start of a run with a large value so
    /// every later `set_heating_current` is the limiting factor.
    async fn set_heating_voltage(&self, voltage: f64) -> Result<(), HardwareError>;

    /// Enables or disables the output.
    async fn set_output_enabled(&self, enabled: bool) -> Result<(), HardwareError>;

    /// Sets the direction of the current through the filament via the relays.
    ///
    /// IMPORTANT: The output must be disabled before the relays are switched,
    /// otherwise the contacts will arc. Implementations must enforce this rather
    /// than trusting the caller.
    async fn set_polarity(&self, polarity: Polarity) -> Result<(), HardwareError>;

    /// Reads every value shown in the filament block in a single pass.
    ///
    /// `get_filament_voltage` and `get_heating_current` exist separately because
    /// a measurement takes its samples in quick succession, far faster than the
    /// one per second this provides.
    async fn snapshot(&self) -> Result<FilamentSnapshot, HardwareError>;
}

/// A vacuum system: a pressure gauge and a turbomolecular pump.
#[async_trait]
pub trait VacuumSystem: std::fmt::Debug + Send + Sync {
    /// Turns the TMP on or off.
    ///
    /// Turning it on must be rejected unless the chamber pressure is below
    /// `TMP_MAXIMUM_BACKING_PRESSURE_MBAR`. Implementations must enforce this
    /// rather than trusting the caller.
    ///
    /// IMPORTANT: Callers must still confirm that the roughing pump is running
    /// before turning the TMP on, otherwise it may be damaged. Nothing here can
    /// check that: a low chamber pressure doesn't prove the pump is still
    /// running, only that it was.
    async fn set_tmp_running(&self, running: bool) -> Result<(), HardwareError>;

    /// Reads every value shown in the vacuum block in a single pass.
    ///
    /// There's no single-value read to match the filament system's: nothing
    /// wants a one-off vacuum reading, because the procedure waits on the
    /// snapshot stream instead.
    async fn snapshot(&self) -> Result<VacuumSnapshot, HardwareError>;
}

/// Everything the application can talk to.
///
/// Field-only: with `snapshot` and `enter_safe_state` on the traits themselves,
/// this exists solely so functions take one parameter instead of two.
///
/// Two separate `Arc`s rather than one combined trait means the real vacuum
/// system can be paired with a mock filament system, or the reverse. `--mock` is
/// a single flag; partial mocking is a constructor choice in `build_hardware`.
#[derive(Clone, Debug)]
pub struct Hardware {
    pub filament: Arc<dyn FilamentSystem>,
    pub vacuum: Arc<dyn VacuumSystem>,
}

/// Polls the hardware every `POLL_INTERVAL` and publishes what it reads.
///
/// A failed poll is logged and retried on the next tick. Returns the error once
/// `POLL_FAILURE_TOLERANCE` polls in a row have failed, which is fatal: the
/// caller reports it and the application shuts down.
///
/// Returns rather than reporting the failure itself so that this module doesn't
/// depend on the application's event type.
///
/// Borrows `snapshots` rather than taking it, so the caller decides when the
/// channel closes. Closing it is how waiters learn the poll has stopped, and
/// they shouldn't learn that before the failure itself has been reported.
pub async fn poll(
    hardware: Hardware,
    snapshots: &watch::Sender<Option<Snapshots>>,
) -> HardwareError {
    let mut ticker = tokio::time::interval(POLL_INTERVAL);

    // The first tick is immediate, so the blocks fill as soon as the
    // instruments answer. A slow poll delays the next rather than bursting to
    // catch up, since a poll that ran late is already up to date.
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let mut failures = 0;

    loop {
        ticker.tick().await;

        // One after the other rather than concurrently: if one failed while the
        // other was mid-transaction, dropping the other's future would release
        // its lock while its I/O carried on, which can split the scope's
        // arm-then-query sequence. Not worth it for a once-a-second poll.
        let result = async {
            Ok::<_, HardwareError>(Snapshots {
                vacuum: hardware.vacuum.snapshot().await?,
                filament: hardware.filament.snapshot().await?,
            })
        }
        .await;

        match result {
            Ok(snapshot) => {
                failures = 0;
                debug!("{:?}", snapshot);

                // An error only means every receiver has gone, which happens
                // as the application shuts down.
                let _ = snapshots.send(Some(snapshot));
            }
            Err(e) => {
                failures += 1;
                if failures >= POLL_FAILURE_TOLERANCE {
                    error!(
                        "polling the hardware failed {} times in a row: {}",
                        failures, e
                    );
                    return e;
                }
                warn!(
                    "polling the hardware failed ({} of {}), retrying: {}",
                    failures, POLL_FAILURE_TOLERANCE, e
                );
            }
        }
    }
}
