use crate::controller::Controller;
use common::ControllerError;
use log::*;
use std::{fmt::Display, sync::Arc};
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

    /// The method hasn't been implemented yet.
    #[error("not implemented")]
    NotImplemented,

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
    /// The channel driving the filament.
    channel: u8,

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
    pub async fn new(channel: u8, controller: Arc<Controller>) -> Result<Self, PowerSupplyError> {
        let device = UsbTmcDevice::open(VENDOR_ID, PRODUCT_ID, None)
            .await
            .inspect_err(|e| error!("failed to open the power supply: {}", e))?;

        Ok(Self {
            state: tokio::sync::Mutex::new(PowerSupplyState {
                channel,
                controller,
                device,
            }),
        })
    }

    /// Gets the current flowing out of the channel in amperes.
    pub async fn get_current(&self) -> Result<f64, PowerSupplyError> {
        // TODO: query ":MEASure:CURRent? CH<channel>" and parse the response.
        error!("power supply get_current isn't implemented");
        Err(PowerSupplyError::NotImplemented)
    }

    /// Gets whether the channel's output is enabled.
    pub async fn get_output_enabled(&self) -> Result<bool, PowerSupplyError> {
        // TODO: query ":OUTPut? CH<channel>" and parse the response.
        error!("power supply get_output_enabled isn't implemented");
        Err(PowerSupplyError::NotImplemented)
    }

    /// Gets the direction of the current through the filament.
    pub async fn get_polarity(&self) -> Result<Polarity, PowerSupplyError> {
        // TODO: send a `Destination::RLY` command whose payload is "?" and map
        // the relay bit mask it returns — 0 to `Nil`, 1 to `Forward`, 2 to
        // `Reverse`, and 3 to `Nil` too, since both relays energised puts both
        // sides of the filament on the positive rail, which is electrically
        // the same as neither and is never deliberately set.
        error!("power supply get_polarity isn't implemented");
        Err(PowerSupplyError::NotImplemented)
    }

    /// Sets the channel's current limit in amperes.
    pub async fn set_current(&self, _current: f64) -> Result<(), PowerSupplyError> {
        // TODO: send ":SOURce<channel>:CURRent <current>".
        error!("power supply set_current isn't implemented");
        Err(PowerSupplyError::NotImplemented)
    }

    /// Enables or disables the channel's output.
    pub async fn set_output_enabled(&self, _enabled: bool) -> Result<(), PowerSupplyError> {
        // TODO: send ":OUTPut CH<channel>,ON" or ":OUTPut CH<channel>,OFF".
        error!("power supply set_output_enabled isn't implemented");
        Err(PowerSupplyError::NotImplemented)
    }

    /// Sets the direction of the current through the filament via the relays.
    ///
    /// IMPORTANT: The output must be disabled before the relays are switched,
    /// otherwise the contacts will arc.
    pub async fn set_polarity(&self, _polarity: Polarity) -> Result<(), PowerSupplyError> {
        // TODO: send a `Destination::RLY` command whose payload is the relay
        // bit mask as a single digit — 0 for `Nil`, 1 for `Forward` (relay 1
        // energised) and 2 for `Reverse` (relay 2 energised).
        error!("power supply set_polarity isn't implemented");
        Err(PowerSupplyError::NotImplemented)
    }

    /// Sets the channel's voltage limit in volts.
    pub async fn set_voltage(&self, _voltage: f64) -> Result<(), PowerSupplyError> {
        // TODO: send ":SOURce<channel>:VOLTage <voltage>".
        error!("power supply set_voltage isn't implemented");
        Err(PowerSupplyError::NotImplemented)
    }
}
