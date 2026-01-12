use crate::controller::{Controller, Destination};
use common::ControllerError;
use log::*;
use std::sync::Arc;
use thiserror::Error;

/// An error returned from the ADC.
///
/// See section 13.1 of the Edwards ADC manual for detailed error descriptions.
#[derive(Debug, Error)]
pub enum AdcError {
    /// An error returned from the vacuum system controller.
    ///
    /// Note that this isn't an error returned from the ADC, but may be
    /// encountered when we try to communicate with it (e.g. a timeout).
    #[error("controller error: {0}")]
    Controller(#[from] ControllerError),

    // Controller errors
    #[error("eeprom checksum failed")]
    Eeprom,
    #[error("the reference used for identifying gauges is incorrect")]
    IdReference,

    // Gauge errors
    #[error("no error")]
    Ok,
    #[error("gauge voltage too high")]
    GaugeVoltageTooHigh,
    #[error("gauge voltage too low")]
    GaugeVoltageTooLow,
    #[error("aim gauge not striking")]
    AimGaugeNotStriking,
    #[error("wrg pirani failure")]
    WrgPirani,
    #[error("wrg inverted magnetron contaminated or short circuited")]
    WrgInvertedMagnetronContaminatedOrShorted,
    #[error("wrg striker filament broken")]
    WrgStrikerFilamentBroken,
    #[error("wrg inverted magnetron not struck")]
    WrgInvertedMagnetronNotStruck,
    #[error("apgx filament broken")]
    ApgxFilamentBroken,
    #[error("apgx calibration error")]
    ApgxCalibration,
    #[error("apgx tube disconnected")]
    ApgxTubeDisconnected,

    // RS-232 errors
    #[error("invalid query or command")]
    InvalidQueryOrCommand,
    #[error("message incomplete")]
    MessageIncomplete,
    #[error("message too long")]
    MessageTooLong,
    #[error("invalid gauge number")]
    InvalidGaugeNumber,
    #[error("invalid number format")]
    InvalidNumberFormat,
    #[error("invalid pressure format")]
    InvalidPressureFormat,
    #[error("no gauge connected")]
    NoGaugeConnected,
    #[error("unknown gauge type")]
    UnknownGaugeType,
    #[error("gauge not reading pressure")]
    GaugeNotReadingPressure,
    #[error("aim gauge striking")]
    AimGaugeStriking,
    #[error("invalid gauge type")]
    InvalidGaugeType,
    #[error("gauge turn-on inhibited by link")]
    GaugeTurnOnInhibitedByLink,

    // Custom errors
    #[error("invalid pressure unit: {0}")]
    InvalidPressureUnit(String),

    /// A catch all error for anything else we might receive.
    #[error("unknown error: {0}")]
    Unknown(String),
}

impl AdcError {
    fn from_error_number(error_number: &str) -> AdcError {
        match error_number {
            // Controller errors
            "00" => AdcError::Ok,
            "01" => AdcError::Eeprom,
            "02" => AdcError::IdReference,

            // Gauge errors
            "11" => AdcError::GaugeVoltageTooHigh,
            "12" => AdcError::GaugeVoltageTooLow,
            "13" => AdcError::AimGaugeNotStriking,
            "21" => AdcError::WrgPirani,
            "22" => AdcError::WrgInvertedMagnetronContaminatedOrShorted,
            "23" => AdcError::WrgStrikerFilamentBroken,
            "24" => AdcError::WrgInvertedMagnetronNotStruck,
            "25" => AdcError::ApgxFilamentBroken,
            "26" => AdcError::ApgxCalibration,
            "27" => AdcError::ApgxTubeDisconnected,

            // RS-232 errors
            "51" => AdcError::InvalidQueryOrCommand,
            "52" => AdcError::MessageIncomplete,
            "53" => AdcError::MessageTooLong,
            "54" => AdcError::InvalidGaugeNumber,
            "57" => AdcError::InvalidNumberFormat,
            "58" => AdcError::InvalidPressureFormat,
            "81" => AdcError::NoGaugeConnected,
            "82" => AdcError::UnknownGaugeType,
            "83" => AdcError::GaugeNotReadingPressure,
            "84" => AdcError::AimGaugeStriking,
            "90" => AdcError::InvalidGaugeType,
            "91" => AdcError::GaugeTurnOnInhibitedByLink,

            _ => AdcError::Unknown(error_number.to_string()),
        }
    }
}

/// The unit in which pressure is being reported by the ADC.
#[derive(Clone, Copy, Debug)]
pub enum PressureUnit {
    Millibar,
    Pascal,
    Torr,
    Volt,
}

/// A pressure measurement, e.g. 1.00 x 10^3 mbar.
#[derive(Clone, Copy, Debug)]
pub struct Pressure {
    pub unit: PressureUnit,
    pub value: f64,
}

struct AdcState {
    controller: Arc<Controller>,
    /// The pressure unit currently being used by the ADC.
    ///
    /// Starts as `None` and is set when we first read/write the units.
    pressure_unit: Option<PressureUnit>,
}

pub struct Adc {
    state: tokio::sync::Mutex<AdcState>,
}

/// Interacts with the Edwards ADC MkII pressure gauge controller.
///
/// Only some commands and queries are implemented. See the controller manual
/// for a list of all supported commands and queries.
impl Adc {
    pub fn new(controller: Arc<Controller>) -> Adc {
        Self {
            state: tokio::sync::Mutex::new(AdcState {
                controller,
                pressure_unit: None,
            }),
        }
    }

    /// Gets the pressure currently reported by the specified gauge number.
    ///
    /// The pressure is returned in the ADC's current units.
    pub async fn get_pressure(&self, gauge_number: u8) -> Result<Pressure, AdcError> {
        let mut state = self.state.lock().await;

        // If we don't know the ADC's current pressure units, query them.
        if state.pressure_unit.is_none() {
            let response = self.send_command(&state.controller, "?US").await?;
            let pressure_unit = match response.as_str() {
                "0" => PressureUnit::Volt,
                "1" => PressureUnit::Millibar,
                "2" => PressureUnit::Pascal,
                "3" => PressureUnit::Torr,
                _ => {
                    error!("unexpected response when reading adc units: {}", response);
                    return Err(AdcError::InvalidPressureUnit(response));
                }
            };
            state.pressure_unit = Some(pressure_unit);
        }

        let response = self
            .send_command(&state.controller, &format!("?GA{}", gauge_number))
            .await?;

        let unit = state.pressure_unit.unwrap();
        let value = match unit {
            // Pressures in volts have a different format.
            PressureUnit::Volt => response
                .parse::<f64>()
                .map_err(|_| AdcError::InvalidPressureFormat)?,

            // All other pressures are the same format.
            _ => {
                if response.len() != 8 {
                    error!("invalid pressure format: {}", response);
                    return Err(AdcError::InvalidPressureFormat);
                }

                let mantissa = response[..4]
                    .parse::<f64>()
                    .map_err(|_| AdcError::InvalidPressureFormat)?;
                let exponent = response[5..]
                    .parse::<i32>()
                    .map_err(|_| AdcError::InvalidPressureFormat)?;

                mantissa * (10.0 as f64).powi(exponent)
            }
        };

        Ok(Pressure { unit, value })
    }

    /// Sets the ADC's current pressure unit.
    pub async fn set_pressure_unit(&self, pressure_unit: PressureUnit) -> Result<(), AdcError> {
        let mut state = self.state.lock().await;
        let unit_number = match pressure_unit {
            PressureUnit::Volt => "0",
            PressureUnit::Millibar => "1",
            PressureUnit::Pascal => "2",
            PressureUnit::Torr => "3",
        };
        let response = self
            .send_command(&state.controller, &format!("!US{}", unit_number))
            .await;
        match response {
            // We expect and Err00 (no error) response. Receiving an `Ok` result
            // would be unexpected and we should report it as an error.
            Ok(response) => {
                error!("unexpected response when setting adc units: {}", response);
                Err(AdcError::Unknown(response))
            }
            Err(AdcError::Ok) => {
                state.pressure_unit = Some(pressure_unit);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    async fn send_command(
        &self,
        controller: &Controller,
        command: &str,
    ) -> Result<String, AdcError> {
        let response = controller.send_command(Destination::ADC, command).await?;
        if response.starts_with("Err") {
            return Err(AdcError::from_error_number(&response[3..5]));
        }
        Ok(response)
    }
}
