//! Canned implementations, so the application runs with no rig attached.
//!
//! These are **not** a simulation. Every getter returns a constant and every
//! setter stores what it was given, so a snapshot reflects what was set. No time
//! dependence, no noise, no physics.
//!
//! The contracts the traits define are kept, because they're contracts rather
//! than simulation: a heating current above the maximum is rejected, the
//! polarity can't be switched while the output is enabled, and the TMP can't be
//! started above its maximum backing pressure.

use super::{FilamentSnapshot, FilamentSystem, HardwareError, VacuumSnapshot, VacuumSystem};
use crate::constants::{MAXIMUM_HEATING_CURRENT_AMPS, TMP_MAXIMUM_BACKING_PRESSURE_MBAR};
use async_trait::async_trait;
use host::{
    adc::{Pressure, PressureUnit},
    power_supply::Polarity,
};
use log::*;
use std::sync::{Mutex, MutexGuard};

/// The voltage across the filament in volts.
const FILAMENT_VOLTAGE: f64 = 0.4821;

/// The chamber pressure in millibar.
const PRESSURE_MBAR: f64 = 1.2e-5;

/// The current draw of the TMP in amperes.
const TMP_CURRENT_AMPS: f32 = 0.42;

/// The rotation speed the TMP reports, and its target, in hertz.
const TMP_ROTATION_SPEED_HERTZ: u16 = 1500;

/// The filament system, with canned readings.
#[derive(Debug, Default)]
pub struct MockFilamentSystem {
    state: Mutex<MockFilamentState>,
}

/// What the setters have been given.
#[derive(Debug, Default)]
struct MockFilamentState {
    heating_current: f64,
    output_enabled: bool,
    polarity: Polarity,
}

impl MockFilamentSystem {
    /// The values the setters have been given.
    ///
    /// Panics only if a previous holder panicked while holding the lock, which
    /// the critical sections here — field assignments and reads — can't do.
    fn state(&self) -> MutexGuard<'_, MockFilamentState> {
        self.state.lock().expect("the mock's mutex was poisoned")
    }
}

#[async_trait]
impl FilamentSystem for MockFilamentSystem {
    async fn enter_safe_state(&self) {
        let mut state = self.state();
        state.heating_current = 0.0;
        state.output_enabled = false;
        state.polarity = Polarity::Nil;
        info!("Mock filament system entered a safe state");
    }

    async fn get_filament_voltage(&self) -> Result<f64, HardwareError> {
        Ok(FILAMENT_VOLTAGE)
    }

    async fn get_heating_current(&self) -> Result<f64, HardwareError> {
        Ok(self.state().heating_current)
    }

    async fn set_heating_current(&self, current: f64) -> Result<(), HardwareError> {
        if current > MAXIMUM_HEATING_CURRENT_AMPS {
            error!("refusing to set a heating current of {} A", current);
            return Err(HardwareError::Other(format!(
                "heating current must be at most {} A: {}",
                MAXIMUM_HEATING_CURRENT_AMPS, current
            )));
        }

        self.state().heating_current = current;
        Ok(())
    }

    async fn set_heating_voltage(&self, _voltage: f64) -> Result<(), HardwareError> {
        // Nothing reads the voltage limit back, so there's nothing to store.
        Ok(())
    }

    async fn set_output_enabled(&self, enabled: bool) -> Result<(), HardwareError> {
        self.state().output_enabled = enabled;
        Ok(())
    }

    async fn set_polarity(&self, polarity: Polarity) -> Result<(), HardwareError> {
        let mut state = self.state();

        if state.output_enabled {
            error!("refusing to switch the relays while the output is enabled");
            return Err(HardwareError::Other(String::from(
                "the output must be disabled before switching polarity",
            )));
        }

        state.polarity = polarity;
        Ok(())
    }

    async fn snapshot(&self) -> Result<FilamentSnapshot, HardwareError> {
        let state = self.state();
        Ok(FilamentSnapshot {
            filament_voltage: FILAMENT_VOLTAGE,
            heating_current: state.heating_current,
            output_enabled: state.output_enabled,
            polarity: state.polarity,
        })
    }
}

/// The vacuum system, with canned readings.
#[derive(Debug, Default)]
pub struct MockVacuumSystem {
    /// What `set_tmp_running` has been given.
    tmp_running: Mutex<bool>,
}

impl MockVacuumSystem {
    /// Whether `set_tmp_running` was last given `true`.
    ///
    /// Panics only if a previous holder panicked while holding the lock, which
    /// the critical sections here can't do.
    fn tmp_running(&self) -> MutexGuard<'_, bool> {
        self.tmp_running
            .lock()
            .expect("the mock's mutex was poisoned")
    }
}

#[async_trait]
impl VacuumSystem for MockVacuumSystem {
    async fn set_tmp_running(&self, running: bool) -> Result<(), HardwareError> {
        if running && PRESSURE_MBAR >= TMP_MAXIMUM_BACKING_PRESSURE_MBAR {
            error!("refusing to start the TMP at {:.1e} mbar", PRESSURE_MBAR);
            return Err(HardwareError::Other(format!(
                "the pressure must be below {} mbar to start the TMP: {:.1e} mbar",
                TMP_MAXIMUM_BACKING_PRESSURE_MBAR, PRESSURE_MBAR
            )));
        }

        *self.tmp_running() = running;
        Ok(())
    }

    async fn snapshot(&self) -> Result<VacuumSnapshot, HardwareError> {
        let tmp_running = *self.tmp_running();

        Ok(VacuumSnapshot {
            pressure: Pressure {
                unit: PressureUnit::Millibar,
                value: PRESSURE_MBAR,
            },
            tmp_current: TMP_CURRENT_AMPS,
            // Follows what `set_tmp_running` was given, instantly: a snapshot
            // reflects what was set, and there's no physics here.
            tmp_current_rotation_speed: if tmp_running {
                TMP_ROTATION_SPEED_HERTZ
            } else {
                0
            },
            tmp_running,
            tmp_target_rotation_speed: TMP_ROTATION_SPEED_HERTZ,
        })
    }
}
