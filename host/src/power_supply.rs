use crate::controller::{Controller, Destination};
use common::ControllerError;
use log::*;
use std::{fmt::Display, num::ParseFloatError, sync::Arc};
use thiserror::Error;
use usb_tmc::UsbTmcDevice;

/// The USB product ID of the Rigol DP-932E power supply.
///
/// Shared by the whole DP900 series.
const PRODUCT_ID: u16 = 0xa4a8;

/// The USB vendor ID of the Rigol DP-932E power supply.
const VENDOR_ID: u16 = 0x1ab1;

/// An error returned from the power supply.
#[derive(Debug, Error)]
pub enum PowerSupplyError {
    /// An error returned from the vacuum system controller.
    ///
    /// Note that this isn't an error returned from the power supply, but may be
    /// encountered when switching the polarity relays, which are driven by the
    /// controller rather than by the supply.
    #[error("controller error: {0}")]
    Controller(#[from] ControllerError),

    #[error("invalid float: {0}")]
    InvalidFloatError(#[from] ParseFloatError),

    /// The method hasn't been implemented yet.
    #[error("not implemented")]
    NotImplemented,

    /// The controller returned an unknown relay polarity.
    #[error("unknown relay polarity: {0}")]
    UnknownRelayPolarity(String),

    /// An error encountered while communicating with the supply over USBTMC.
    #[error("usb-tmc error: {0}")]
    UsbTmc(#[from] usb_tmc::Error),
}

/// The direction of the current through the filament.
///
/// Each side of the filament is switched by one SPDT relay. With both relays
/// de-energised, both sides sit on the negative rail, no current can flow, and
/// the current has no direction — that's `Nil`. That is the state the
/// controller powers up in, and the state cleanup returns to. Energising one
/// relay or the other puts one side on the positive rail, setting the
/// direction.
///
/// Reversing the direction lets the Seebeck voltages at the filament's
/// junctions cancel when a forward and a reverse measurement are averaged.
///
/// A fourth relay state exists — both energised, both sides on the positive
/// rail — which is equivalent to `Nil` and is never used.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Polarity {
    /// Relay 1 energised: current flows through the filament one way.
    Forward,

    /// Both relays de-energised: no current flows, so there is no polarity.
    #[default]
    Nil,

    /// Relay 2 energised: current flows the other way.
    Reverse,
}

impl Display for Polarity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Polarity::Forward => f.write_str("Forward"),
            Polarity::Nil => f.write_str("Nil"),
            Polarity::Reverse => f.write_str("Reverse"),
        }
    }
}

// TODO: remove this once the methods below are implemented and read these
// fields.
#[allow(dead_code)]
#[derive(Debug)]
struct PowerSupplyState {
    /// Used to switch the SPDT polarity relays.
    ///
    /// The relays are wired to the controller rather than to the supply, so
    /// changing polarity goes over the serial link and not over USBTMC.
    controller: Arc<Controller>,

    /// The supply itself.
    device: UsbTmcDevice,
}

/// Interacts with the Rigol DP-932E power supply and the SPDT polarity relays.
///
/// Always uses the power supply's first channel.
///
/// Only some commands and queries are implemented. See the supply's
/// programming guide for a list of all supported commands and queries.
#[derive(Debug)]
pub struct PowerSupply {
    // TODO: remove this once the methods below read the state.
    #[allow(dead_code)]
    state: tokio::sync::Mutex<PowerSupplyState>,
}

impl PowerSupply {
    /// `controller` is used to switch the SPDT polarity relays.
    ///
    /// The supply itself is found by its USB vendor and product IDs.
    pub async fn new(controller: Arc<Controller>) -> Result<Self, PowerSupplyError> {
        let device = UsbTmcDevice::open(VENDOR_ID, PRODUCT_ID, None)
            .await
            .inspect_err(|e| error!("failed to open the power supply: {}", e))?;

        Ok(Self {
            state: tokio::sync::Mutex::new(PowerSupplyState { controller, device }),
        })
    }

    /// Gets the current flowing out of channel 1 in amperes.
    pub async fn get_current(&self) -> Result<f64, PowerSupplyError> {
        let state = self.state.lock().await;
        Ok(state
            .device
            .query_str(":MEASure:CURRent? CH1")
            .await?
            .trim()
            .parse()?)
    }

    /// Gets whether channel 1's output is enabled.
    pub async fn get_output_enabled(&self) -> Result<bool, PowerSupplyError> {
        let state = self.state.lock().await;
        Ok(state.device.query_str(":OUTPut? CH1").await?.trim() == "1")
    }

    /// Gets the direction of the current through the filament.
    pub async fn get_polarity(&self) -> Result<Polarity, PowerSupplyError> {
        let state = self.state.lock().await;
        let polarity = state
            .controller
            .send_command(Destination::RLY, "?")
            .await?
            .trim()
            .to_string();
        match polarity.as_str() {
            "0" | "3" => Ok(Polarity::Nil),
            "1" => Ok(Polarity::Forward),
            "2" => Ok(Polarity::Reverse),
            _ => Err(PowerSupplyError::UnknownRelayPolarity(polarity)),
        }
    }

    /// Sets the channel's current limit in amperes.
    pub async fn set_current_limit(&self, current: f64) -> Result<(), PowerSupplyError> {
        let state = self.state.lock().await;
        state
            .device
            .write_str(format!(":SOURce1:CURRent {}", current).as_str())
            .await?;
        Ok(())
    }

    /// Enables or disables the channel's output.
    pub async fn set_output_enabled(&self, enabled: bool) -> Result<(), PowerSupplyError> {
        let state = self.state.lock().await;
        state
            .device
            .write_str(format!(":OUTPut CH1,{}", enabled as u8).as_str())
            .await?;
        Ok(())
    }

    /// Sets the direction of the current through the filament via the relays.
    ///
    /// IMPORTANT: The output must be disabled before the relays are switched,
    /// otherwise the contacts may arc.
    pub async fn set_polarity(&self, polarity: Polarity) -> Result<(), PowerSupplyError> {
        let state = self.state.lock().await;
        state
            .controller
            .send_command(
                Destination::RLY,
                match polarity {
                    Polarity::Nil => "0",
                    Polarity::Forward => "1",
                    Polarity::Reverse => "2",
                },
            )
            .await?;
        Ok(())
    }

    /// Sets the channel's voltage limit in volts.
    pub async fn set_voltage_limit(&self, voltage: f64) -> Result<(), PowerSupplyError> {
        let state = self.state.lock().await;
        state
            .device
            .write_str(format!(":SOURce1:VOLTage {}", voltage).as_str())
            .await?;
        Ok(())
    }
}
