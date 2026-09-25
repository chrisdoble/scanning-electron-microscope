//! The characterisation procedure itself.
//!
//! The top-level sections of the run, in order, each an ordinary async function
//! so the procedure reads as the sequence of things that happen. Every section
//! takes the `Hardware`: that's how one reads an instrument more often than the
//! once-a-second snapshot, as `measure_at` does for its samples.
//!
//! TODO: the measurements are stubs. `measure_cold_resistance` runs the whole
//! measurement pattern end to end — samples, both polarities, Python — but at a
//! placeholder current and settling time, and the real procedure goes further.

use super::{Context, ProcedureError};
use crate::{
    constants::{MAXIMUM_HEATING_VOLTAGE_VOLTS, TMP_MAXIMUM_BACKING_PRESSURE_MBAR},
    hardware::Hardware,
    python,
    results::Measurement,
};
use host::power_supply::Polarity;
use std::time::Duration;

/// The current the cold resistance is measured at in amperes.
///
/// A stub value, half `MAXIMUM_HEATING_CURRENT_AMPS` so that it stays within
/// the cap if the cap moves.
const COLD_RESISTANCE_CURRENT_AMPS: f64 = 0.05;

/// How many samples each measured quantity is summarised from.
const SAMPLE_COUNT: usize = 20;

/// How long the current is left to settle before sampling. A stub value.
const SETTLING_TIME: Duration = Duration::from_secs(1);

/// Characterises a filament.
pub async fn characterise(ctx: Context, hardware: Hardware) -> Result<(), ProcedureError> {
    ctx.section("Preparing", |ctx| prepare(ctx, hardware.clone()))
        .await?;
    ctx.section("Pumping down chamber", |ctx| {
        pump_down(ctx, hardware.clone())
    })
    .await?;
    ctx.section("Measuring cold resistance", |ctx| {
        measure_cold_resistance(ctx, hardware.clone())
    })
    .await?;
    ctx.section("Spinning down TMP", |ctx| spin_down(ctx, hardware.clone()))
        .await?;
    ctx.section("Finishing", |ctx| finish(ctx, hardware.clone()))
        .await?;
    Ok(())
}

/// Identifies the filament and readies the supply and the chamber.
async fn prepare(ctx: Context, hardware: Hardware) -> Result<(), ProcedureError> {
    // First, before anything is measured, so no results file with a
    // measurement in it can lack one. The first save is also what tells the
    // operator where the results are going.
    let filament_id: String = ctx.input("Filament ID", None).await?;
    ctx.record(|characterisation| characterisation.filament_id = Some(filament_id));
    ctx.save();

    // With the voltage limit high, the current limit is the one that binds, so
    // the channel runs in constant current.
    hardware
        .filament
        .set_heating_voltage(MAXIMUM_HEATING_VOLTAGE_VOLTS)
        .await?;
    ctx.text(format!(
        "Set the heating voltage limit to {:.1} V",
        MAXIMUM_HEATING_VOLTAGE_VOLTS
    ));

    ctx.confirm("Confirm that the filament is mounted").await?;
    ctx.confirm("Confirm that the chamber is sealed").await?;
    Ok(())
}

/// Brings the chamber down to the TMP's base pressure.
async fn pump_down(ctx: Context, hardware: Hardware) -> Result<(), ProcedureError> {
    // Nothing can check this: a low chamber pressure only proves the roughing
    // pump was running, not that it still is.
    ctx.confirm("Confirm that the roughing pump is running")
        .await?;

    ctx.wait_for(
        "Waiting for the chamber to reach the TMP's operating pressure",
        |s| s.vacuum.pressure.value < TMP_MAXIMUM_BACKING_PRESSURE_MBAR,
    )
    .await?;

    hardware.vacuum.set_tmp_running(true).await?;
    ctx.text("Turned on the TMP");

    ctx.wait_for("Waiting for the TMP to reach speed", |s| {
        s.vacuum.tmp_current_rotation_speed >= s.vacuum.tmp_target_rotation_speed
    })
    .await?;

    ctx.measurement(
        "Base pressure",
        format!("{:.1e} mbar", ctx.snapshot().vacuum.pressure.value),
    );
    Ok(())
}

/// Measures the filament's voltage and current at a small current, in both
/// directions.
///
/// Reversing the direction lets the Seebeck voltages at the filament's
/// junctions cancel when the two are averaged. This is the reference for the
/// measurement pattern: samples, summarised by Python, recorded and saved.
async fn measure_cold_resistance(ctx: Context, hardware: Hardware) -> Result<(), ProcedureError> {
    let (voltage, current) = ctx
        .section("Forward polarity", |ctx| {
            measure_at(ctx, hardware.clone(), Polarity::Forward)
        })
        .await?;
    ctx.record(|characterisation| {
        characterisation.cold_forward_current_amps = Some(current);
        characterisation.cold_forward_voltage_volts = Some(voltage);
    });
    ctx.save();

    let (voltage, current) = ctx
        .section("Reverse polarity", |ctx| {
            measure_at(ctx, hardware.clone(), Polarity::Reverse)
        })
        .await?;
    ctx.record(|characterisation| {
        characterisation.cold_reverse_current_amps = Some(current);
        characterisation.cold_reverse_voltage_volts = Some(voltage);
    });
    ctx.save();

    Ok(())
}

/// Takes `SAMPLE_COUNT` samples of the filament's voltage and current with the
/// current flowing in `polarity`'s direction, returning the voltage and the
/// current in that order.
async fn measure_at(
    ctx: Context,
    hardware: Hardware,
    polarity: Polarity,
) -> Result<(Measurement, Measurement), ProcedureError> {
    let filament = &hardware.filament;

    // The relays can only be switched with the output off. Don't assume it
    // already is: on the first measurement it's however the supply was left.
    filament.set_output_enabled(false).await?;
    filament.set_polarity(polarity).await?;
    filament
        .set_heating_current(COLD_RESISTANCE_CURRENT_AMPS)
        .await?;
    filament.set_output_enabled(true).await?;
    ctx.text(format!(
        "Enabled the output at {:.3} A",
        COLD_RESISTANCE_CURRENT_AMPS
    ));

    ctx.waiting("Waiting for the current to settle", || async {
        tokio::time::sleep(SETTLING_TIME).await;
        Ok(())
    })
    .await?;

    // Read directly rather than from the snapshots: these are taken as fast as
    // the instruments answer, far faster than the poll.
    let (voltages, currents) = ctx
        .waiting(format!("Taking {} samples", SAMPLE_COUNT), || async {
            let mut voltages = Vec::with_capacity(SAMPLE_COUNT);
            let mut currents = Vec::with_capacity(SAMPLE_COUNT);
            for _ in 0..SAMPLE_COUNT {
                voltages.push(filament.get_filament_voltage().await?);
                currents.push(filament.get_heating_current().await?);
            }
            Ok((voltages, currents))
        })
        .await?;

    filament.set_output_enabled(false).await?;
    ctx.text("Disabled the output");

    let voltage = summarise(voltages).await?;
    let current = summarise(currents).await?;
    ctx.measurement(
        "Voltage",
        format!(
            "{:.3} ± {:.3} mV",
            voltage.value * 1000.0,
            voltage.uncertainty * 1000.0
        ),
    );
    ctx.measurement(
        "Current",
        format!(
            "{:.3} ± {:.3} mA",
            current.value * 1000.0,
            current.uncertainty * 1000.0
        ),
    );

    Ok((voltage, current))
}

/// Stops the TMP and waits for it to come to rest.
async fn spin_down(ctx: Context, hardware: Hardware) -> Result<(), ProcedureError> {
    hardware.vacuum.set_tmp_running(false).await?;
    ctx.text("Turned off the TMP");

    ctx.wait_for("Waiting for the TMP to spin down", |s| {
        s.vacuum.tmp_current_rotation_speed == 0
    })
    .await?;
    Ok(())
}

/// Leaves the filament system unpowered and saves a final time.
async fn finish(ctx: Context, hardware: Hardware) -> Result<(), ProcedureError> {
    let filament = &hardware.filament;

    // The output goes off before the relays are switched, which
    // `set_polarity` insists on.
    filament.set_heating_current(0.0).await?;
    filament.set_output_enabled(false).await?;
    filament.set_polarity(Polarity::Nil).await?;
    ctx.text("Zeroed the current, disabled the output and de-energised the relays");

    ctx.save();
    Ok(())
}

/// The mean and standard error of `samples`, from Python, with the samples kept
/// so the analysis can be redone.
async fn summarise(samples: Vec<f64>) -> Result<Measurement, ProcedureError> {
    let (value, uncertainty) = python::mean_and_standard_error(&samples).await?;
    Ok(Measurement {
        samples,
        uncertainty,
        value,
    })
}
