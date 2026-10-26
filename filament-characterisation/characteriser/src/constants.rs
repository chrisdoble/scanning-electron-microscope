//! The limits the procedure and the hardware adapters work within.

/// The chamber pressure above which the procedure stops if the filament is
/// powered, in mbar.
pub const FILAMENT_ABORT_PRESSURE_MBAR: f64 = 5e-5;

/// The chamber pressure that must be reached before any current flows through
/// the filament, in mbar.
///
/// An arbitrary but comfortable threshold.
pub const FILAMENT_OPERATING_PRESSURE_MBAR: f64 = 1e-5;

/// The power supply's maximum current in amperes: the Rigol DP-932E's maximum
/// per channel.
///
/// The filament is never driven above it, but it only guards against asking
/// for something the supply can't do. The limits that matter for the filament
/// are the hot sweep's maximum and, for the pulse binary, the operator's
/// step-by-step plan.
pub const POWER_SUPPLY_MAXIMUM_CURRENT_AMPS: f64 = 3.0;

/// The power supply's maximum voltage in volts: the DP-932E's channels 1 and 2
/// go to 30 V.
///
/// The filament channel's voltage limit is never set above it, but it's only a
/// general ceiling. Each measurement sets its own, much lower, limit.
pub const POWER_SUPPLY_MAXIMUM_VOLTAGE_VOLTS: f64 = 30.0;

/// The maximum backing pressure supported by the TMP in mbar.
///
/// At pressures above this, the TMP can't safely run.
///
/// The manual for the Preiffer TMH 071 P states a maximum backing pressure of
/// 18 mbar, but I'll use an even lower pressure here just to be safe.
pub const TMP_MAXIMUM_BACKING_PRESSURE_MBAR: f64 = 12.0;
