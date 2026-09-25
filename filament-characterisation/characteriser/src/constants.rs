//! The limits the procedure and the hardware adapters work within.

/// The maximum heating current the filament may be driven with in amperes.
///
/// Deliberately low while the procedure is a stub, so nothing it does can heat
/// a filament even if it's run against the rig.
///
/// TODO: raise this with the real procedure, confirmed against what the
/// filament tolerates. The Rigol DP-932E can supply 3 A per channel, so the
/// supply isn't the binding constraint.
pub const MAXIMUM_HEATING_CURRENT_AMPS: f64 = 0.1;

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
