//! The limits the procedure and the hardware adapters work within.

// TODO: remove this once the procedure reads these (step 8 of the design
// document's build order). The ones the adapters already check are only reached
// from methods that have no caller yet, which counts as unused.
#![allow(dead_code)]

/// The maximum heating current the filament may be driven with in amperes.
///
/// TODO: confirm this against the filament before a real run. The Rigol DP-932E
/// can supply 3 A per channel, so the supply isn't the binding constraint —
/// this value has to come from what the filament tolerates.
pub const MAXIMUM_HEATING_CURRENT_AMPS: f64 = 2.0;

/// The voltage limit the filament channel is set to in volts.
///
/// The supply runs in whichever mode its limits make it, so this is set high
/// once at the start of a run to leave the current limit as the binding one,
/// putting the channel in constant current.
///
/// TODO: confirm this. The DP-932E's channel 1 goes to 32 V, so this is just
/// under its ceiling.
pub const MAXIMUM_HEATING_VOLTAGE_VOLTS: f64 = 30.0;

/// The maximum backing pressure supported by the TMP in mbar.
///
/// At pressures above this, the TMP can't safely run.
///
/// The manual for the Preiffer TMH 071 P states a maximum backing pressure of
/// 18 mbar, but I'll use an even lower pressure here just to be safe.
pub const TMP_MAXIMUM_BACKING_PRESSURE_MBAR: f64 = 12.0;
