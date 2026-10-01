//! Adapters over the `host` instrument types.
//!
//! These map types and errors and enforce the contracts the traits define.
//! No other logic belongs here, and nothing outside this file names a `host`
//! instrument type (plain data like `Polarity` and `Voltages` is fine) — so if a `host` signature changes, this is the only file
//! that follows it.

use super::{FilamentSnapshot, FilamentSystem, HardwareError, VacuumSnapshot, VacuumSystem};
use crate::constants::{
    MAXIMUM_HEATING_CURRENT_AMPS, MAXIMUM_HEATING_VOLTAGE_VOLTS, TMP_MAXIMUM_BACKING_PRESSURE_MBAR,
};
use async_trait::async_trait;
use host::{
    adc::{Adc, PressureUnit},
    controller::Controller,
    oscilloscope::{Oscilloscope, Voltages},
    power_supply::{Polarity, PowerSupply, RegulationMode},
    tmp::Tmp,
};
use log::*;
use std::sync::Arc;

/// The filament system, over the real power supply and oscilloscope.
#[derive(Debug)]
pub struct RealFilamentSystem {
    oscilloscope: Oscilloscope,
    power_supply: PowerSupply,
}

impl RealFilamentSystem {
    /// `controller` is used to switch the polarity relays.
    ///
    /// Both instruments are found by their USB vendor and product IDs, so both
    /// must be attached. Both are reset so they start from a known state: the
    /// oscilloscope's channel, timebase and vertical scale set up for
    /// measuring, and the supply's output off, its limits zero and the relays
    /// de-energised, however it was left.
    pub async fn new(controller: Arc<Controller>) -> Result<Self, HardwareError> {
        let oscilloscope = Oscilloscope::new().await?;
        oscilloscope.reset().await?;

        let power_supply = PowerSupply::new(controller).await?;
        power_supply.reset().await?;

        Ok(Self {
            oscilloscope,
            power_supply,
        })
    }
}

#[async_trait]
impl FilamentSystem for RealFilamentSystem {
    async fn enter_safe_state(&self) -> Result<(), HardwareError> {
        // Every action is attempted even if an earlier one failed, so a single
        // failure can't leave the filament powered. Note that if disabling the
        // output fails, `set_polarity` then refuses to switch the relays — that
        // is the safe outcome, not a bug: better to leave them as they are than
        // to arc the contacts.
        let results = [
            (
                "zero the heating current",
                self.set_heating_current_limit(0.0).await,
            ),
            ("disable the output", self.set_output_enabled(false).await),
            (
                "de-energise the relays",
                self.set_polarity(Polarity::Nil).await,
            ),
        ];

        let mut first = Ok(());
        for (action, result) in results {
            if let Err(e) = result {
                error!("failed to {}: {}", action, e);
                if first.is_ok() {
                    first = Err(HardwareError::Other(format!("couldn't {}: {}", action, e)));
                }
            }
        }
        first
    }

    async fn get_filament_voltages(&self) -> Result<Voltages, HardwareError> {
        Ok(self.oscilloscope.get_voltages().await?)
    }

    async fn get_heating_current(&self) -> Result<f64, HardwareError> {
        Ok(self.power_supply.get_current().await?)
    }

    async fn get_heating_current_limit(&self) -> Result<f64, HardwareError> {
        Ok(self.power_supply.get_current_limit().await?)
    }

    async fn get_overcurrent_tripped(&self) -> Result<bool, HardwareError> {
        Ok(self.power_supply.get_overcurrent_tripped().await?)
    }

    async fn get_regulation_mode(&self) -> Result<RegulationMode, HardwareError> {
        Ok(self.power_supply.get_regulation_mode().await?)
    }

    async fn set_heating_current_limit(&self, current: f64) -> Result<(), HardwareError> {
        if current > MAXIMUM_HEATING_CURRENT_AMPS {
            error!("refusing to set a heating current of {} A", current);
            return Err(HardwareError::Other(format!(
                "heating current must be at most {} A: {}",
                MAXIMUM_HEATING_CURRENT_AMPS, current
            )));
        }

        Ok(self.power_supply.set_current_limit(current).await?)
    }

    async fn set_heating_voltage_limit(&self, voltage: f64) -> Result<(), HardwareError> {
        if voltage > MAXIMUM_HEATING_VOLTAGE_VOLTS {
            error!("refusing to set a heating voltage of {} V", voltage);
            return Err(HardwareError::Other(format!(
                "heating voltage must be at most {} V: {}",
                MAXIMUM_HEATING_VOLTAGE_VOLTS, voltage
            )));
        }

        Ok(self.power_supply.set_voltage_limit(voltage).await?)
    }

    async fn set_overcurrent_protection(&self, current: f64) -> Result<(), HardwareError> {
        Ok(self
            .power_supply
            .set_overcurrent_protection(current)
            .await?)
    }

    async fn set_output_enabled(&self, enabled: bool) -> Result<(), HardwareError> {
        Ok(self.power_supply.set_output_enabled(enabled).await?)
    }

    async fn set_polarity(&self, polarity: Polarity) -> Result<(), HardwareError> {
        // Switching the relays under load arcs their contacts, so check rather
        // than trust the caller.
        if self.power_supply.get_output_enabled().await? {
            error!("refusing to switch the relays while the output is enabled");
            return Err(HardwareError::Other(String::from(
                "the output must be disabled before switching polarity",
            )));
        }

        Ok(self.power_supply.set_polarity(polarity).await?)
    }

    async fn set_vertical_scale(&self, scale: f64) -> Result<(), HardwareError> {
        Ok(self.oscilloscope.set_vertical_scale(scale).await?)
    }

    async fn snapshot(&self) -> Result<FilamentSnapshot, HardwareError> {
        Ok(FilamentSnapshot {
            filament_voltage: match self.oscilloscope.get_voltages().await? {
                Voltages {
                    average: Some(average),
                    maximum: Some(_),
                    minimum: Some(_),
                } => Some(average),
                _ => None,
            },
            heating_current: self.power_supply.get_current().await?,
            output_enabled: self.power_supply.get_output_enabled().await?,
            polarity: self.power_supply.get_polarity().await?,
        })
    }
}

/// The vacuum system, over the real ADC and TMP.
#[derive(Debug)]
pub struct RealVacuumSystem {
    adc: Adc,
    tmp: Tmp,
}

impl RealVacuumSystem {
    /// Both instruments share `controller`, which is the one serial link to the
    /// vacuum hardware.
    ///
    /// The ADC is switched to millibar so that every pressure the application
    /// sees is in the unit its limits are expressed in.
    pub async fn new(
        controller: Arc<Controller>,
        adc_gauge_number: u8,
        tmp_address: &str,
    ) -> Result<Self, HardwareError> {
        let adc = Adc::new(Arc::clone(&controller), adc_gauge_number);
        adc.set_pressure_unit(PressureUnit::Millibar).await?;

        Ok(Self {
            adc,
            tmp: Tmp::new(tmp_address, controller),
        })
    }
}

#[async_trait]
impl VacuumSystem for RealVacuumSystem {
    async fn set_tmp_running(&self, running: bool) -> Result<(), HardwareError> {
        // Starting the TMP against too high a backing pressure can destroy it,
        // so check rather than trust the caller. Stopping it is always allowed.
        if running {
            let pressure = self.adc.get_pressure().await?;

            // The limit is in millibar, and `new` set the ADC to millibar, but
            // a comparison in the wrong unit would be meaningless, so make sure.
            if !matches!(pressure.unit, PressureUnit::Millibar) {
                error!(
                    "refusing to start the TMP: pressure is in {}",
                    pressure.unit
                );
                return Err(HardwareError::Other(format!(
                    "expected the pressure in mbar, got {}",
                    pressure.unit
                )));
            }

            if pressure.value >= TMP_MAXIMUM_BACKING_PRESSURE_MBAR {
                error!("refusing to start the TMP at {:.1e} mbar", pressure.value);
                return Err(HardwareError::Other(format!(
                    "the pressure must be below {} mbar to start the TMP: {:.1e} mbar",
                    TMP_MAXIMUM_BACKING_PRESSURE_MBAR, pressure.value
                )));
            }
        }

        Ok(self.tmp.set_running(running).await?)
    }

    async fn snapshot(&self) -> Result<VacuumSnapshot, HardwareError> {
        Ok(VacuumSnapshot {
            pressure: self.adc.get_pressure().await?,
            tmp_current: self.tmp.get_current().await?,
            tmp_current_rotation_speed: self.tmp.get_current_rotation_speed().await?,
            tmp_running: self.tmp.is_running().await?,
            tmp_target_rotation_speed: self.tmp.get_target_rotation_speed().await?,
        })
    }
}
