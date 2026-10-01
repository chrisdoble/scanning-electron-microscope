//! The limits the procedure and the hardware adapters work within.

/// The chamber pressure above which the procedure stops if the filament is
/// powered, in mbar.
pub const FILAMENT_ABORT_PRESSURE_MBAR: f64 = 5e-5;

/// The chamber pressure that must be reached before any current flows through
/// the filament, in mbar.
///
/// An arbitrary but comfortable threshold.
pub const FILAMENT_OPERATING_PRESSURE_MBAR: f64 = 1e-5;

/// The maximum heating current the filament may be driven with in amperes.
///
/// The largest cold-resistance setpoint, which dissipates only about 10 mW in a
/// 0.1 Ω filament. The Rigol DP-932E can supply 3 A per channel, so the supply
/// isn't the binding constraint.
pub const MAXIMUM_HEATING_CURRENT_AMPS: f64 = 0.3;

/// The maximum voltage limit the filament channel may be set to in volts.
///
/// A general ceiling: the DP-932E's channels 1 and 2 go to 30 V. Each
/// measurement sets its own, much lower, limit.
pub const MAXIMUM_HEATING_VOLTAGE_VOLTS: f64 = 30.0;

/// The maximum backing pressure supported by the TMP in mbar.
///
/// At pressures above this, the TMP can't safely run.
///
/// The manual for the Preiffer TMH 071 P states a maximum backing pressure of
/// 18 mbar, but I'll use an even lower pressure here just to be safe.
pub const TMP_MAXIMUM_BACKING_PRESSURE_MBAR: f64 = 12.0;
