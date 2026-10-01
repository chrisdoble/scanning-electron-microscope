//! The characterisation procedure itself.
//!
//! The top-level sections of the run, in order, each an ordinary async function
//! so the procedure reads as the sequence of things that happen. Every section
//! takes the `Hardware`: that's how one reads an instrument more often than the
//! once-a-second snapshot, as `take_samples` does.
//!
//! TODO: `measure_cold_resistance` is a stub. It exercises the helpers the real
//! procedure is built from — ramping, settling, sampling, reversing — at a
//! single current, and records into the old `cold_*` fields. The real one
//! measures every setpoint and fits them (docs/COLD_RESISTANCE.md).

use super::{Context, ProcedureError};
use crate::{
    constants::{
        FILAMENT_ABORT_PRESSURE_MBAR, FILAMENT_OPERATING_PRESSURE_MBAR,
        TMP_MAXIMUM_BACKING_PRESSURE_MBAR,
    },
    hardware::Hardware,
    python,
    results::Measurement,
};
use host::{
    oscilloscope::Voltages,
    power_supply::{Polarity, RegulationMode},
};
use log::*;
use std::time::{Duration, Instant};

/// A reading counts as clipped if the voltage's peak is more than this
/// fraction of the way to the edge of the screen (4 divisions from the
/// centre).
///
/// The scale is chosen once, then fixed, and later readings are bigger: the
/// filament's resistance keeps rising as it settles, a later window can catch a
/// larger noise peak, and offsets drift. Without margin, a scale that only just
/// fits would clip mid-run and abort it. The margin costs almost nothing,
/// because the gain uncertainty doesn't depend on the scale.
const CLIPPING_HEADROOM: f64 = 0.8;

/// The currents the cold resistance is measured at in amperes.
const COLD_RESISTANCE_SETPOINTS_AMPS: [f64; 9] = [
    0.100, 0.125, 0.150, 0.175, 0.200, 0.225, 0.250, 0.275, 0.300,
];

/// The voltage limit while measuring the cold resistance in volts.
///
/// About 0.15 V is needed. A low limit keeps the channel in constant current
/// while limiting the energy delivered if the circuit opens and re-closes,
/// which 30 V wouldn't.
const COLD_RESISTANCE_VOLTAGE_LIMIT_VOLTS: f64 = 2.0;

/// How long `wait_for_settle` waits before giving up and carrying on.
const MAXIMUM_SETTLE_TIME: Duration = Duration::from_secs(60);

/// How far above the largest setpoint the supply's overcurrent protection is
/// set, in amperes, as a hardware backstop.
///
/// Its 10 ms delay should ride out any overshoot when the output is re-enabled
/// at a setpoint. If it trips anyway, widen this or lengthen the delay.
const OVERCURRENT_PROTECTION_MARGIN_AMPS: f64 = 0.05;

/// How far `ramp_to` moves the current at each step in amperes.
///
/// With `RAMP_STEP_INTERVAL`, 50 mA/s. At cold-resistance currents ramping has
/// no physical benefit; it's there so `ramp_to` is ready for hot measurements,
/// where each jump should be small compared with what the filament's thermal
/// time constant smooths out.
const RAMP_STEP_AMPS: f64 = 0.005;

/// How long `ramp_to` waits between steps.
const RAMP_STEP_INTERVAL: Duration = Duration::from_millis(100);

/// How long the output stays off while the polarity relays move.
const RELAY_SETTLE_TIME: Duration = Duration::from_millis(50);

/// How many samples each measured quantity is summarised from.
const SAMPLE_COUNT: usize = 20;

/// The relative change in resistance between two windows below which the
/// filament counts as settled, whatever the noise.
///
/// About one point's statistical uncertainty in resistance at the lowest
/// setpoint, so a remaining drift is below what the point can resolve anyway.
/// A starting point: tune it from the logged settle times and per-point
/// uncertainties after the first runs on the rig.
const SETTLE_TOLERANCE: f64 = 2e-4;

/// The length of each of the two windows `wait_for_settle` compares. The
/// earliest possible settle is twice this.
const SETTLE_WINDOW: Duration = Duration::from_secs(2);

/// The oscilloscope's vertical scales in volts per division, smallest first.
///
/// Starts at 10 mV/div, because the ±1% gain spec is only "typical" at 5 mV/div
/// and below.
const VERTICAL_SCALES_VOLTS_PER_DIVISION: [f64; 10] =
    [0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0];

/// How `wait_for_settle` finished, and after how long.
enum Settle {
    Settled(Duration),
    TimedOut(Duration),
}

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
    ctx.confirm("Press enter to exit").await?;
    Ok(())
}

/// Identifies the filament and has the operator confirm the rig is ready.
///
/// Sets no limits on the supply: each measurement sets the ones it needs.
async fn prepare(ctx: Context, _hardware: Hardware) -> Result<(), ProcedureError> {
    // First, before anything is measured, so no results file with a
    // measurement in it can lack one. The first save is also what tells the
    // operator where the results are going.
    let filament_id: String = ctx.input("Filament ID", None).await?;
    ctx.record(|characterisation| characterisation.filament_id = Some(filament_id));
    ctx.save();

    ctx.confirm("Confirm that the filament is mounted").await?;
    ctx.confirm("Confirm that the chamber is sealed").await?;

    // The scope's probe ratio is reset to 1×, so this is what makes that
    // correct. A probe switched to 10× would read 10× low.
    ctx.confirm(
        "Confirm that a 1× probe is connected to the oscilloscope's channel 1 and that its switch is set to 1×",
    )
    .await?;

    // The supply's accuracy figures assume a 1-hour warm-up, and the
    // pump-down usually covers it.
    ctx.confirm(
        "Confirm that the oscilloscope has been on for at least 30 minutes and the power supply for at least 60 minutes",
    )
    .await?;
    Ok(())
}

/// Brings the chamber down to the pressure the filament is operated at.
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

    ctx.wait_for(
        "Waiting for the chamber to reach the filament's operating pressure",
        |s| s.vacuum.pressure.value < FILAMENT_OPERATING_PRESSURE_MBAR,
    )
    .await?;

    ctx.measurement(
        "Base pressure",
        format!("{:.1e} mbar", ctx.snapshot().vacuum.pressure.value),
    );
    Ok(())
}

/// Measures the filament's voltage and current at the lowest setpoint, in both
/// directions.
///
/// Reversing the direction lets the Seebeck voltages at the filament's
/// junctions cancel when the two are averaged. A stub for the real procedure:
/// see the module comment.
async fn measure_cold_resistance(ctx: Context, hardware: Hardware) -> Result<(), ProcedureError> {
    let filament = &hardware.filament;
    let current = COLD_RESISTANCE_SETPOINTS_AMPS[0];
    let scale = VERTICAL_SCALES_VOLTS_PER_DIVISION[0];

    let largest_setpoint = COLD_RESISTANCE_SETPOINTS_AMPS
        .iter()
        .copied()
        .fold(f64::MIN, f64::max);
    let overcurrent_protection = largest_setpoint + OVERCURRENT_PROTECTION_MARGIN_AMPS;

    // Don't assume the output is off: it's however the supply was left.
    filament.set_output_enabled(false).await?;
    filament.set_heating_current_limit(0.0).await?;
    filament
        .set_heating_voltage_limit(COLD_RESISTANCE_VOLTAGE_LIMIT_VOLTS)
        .await?;
    filament
        .set_overcurrent_protection(overcurrent_protection)
        .await?;
    ctx.text(format!(
        "Set the voltage limit to {:.1} V and overcurrent protection to {:.3} A",
        COLD_RESISTANCE_VOLTAGE_LIMIT_VOLTS, overcurrent_protection
    ));

    filament.set_vertical_scale(scale).await?;
    ctx.text(format!(
        "Set the vertical scale to {} mV/div",
        scale * 1000.0
    ));

    // The relays are switched with the output off, then the current is ramped
    // up from 0 A.
    filament.set_polarity(Polarity::Forward).await?;
    filament.set_output_enabled(true).await?;
    ramp_to(&ctx, &hardware, current).await?;

    let (voltage, current_measurement) = ctx
        .section("Forward polarity", |ctx| {
            measure(ctx, hardware.clone(), scale)
        })
        .await?;
    ctx.record(|characterisation| {
        characterisation.cold_forward_current_amps = Some(current_measurement);
        characterisation.cold_forward_voltage_volts = Some(voltage);
    });
    ctx.save();

    let (voltage, current_measurement) = ctx
        .section("Reverse polarity", |ctx| {
            let hardware = hardware.clone();
            async move {
                change_polarity(&hardware, Polarity::Reverse).await?;
                ctx.text("Reversed the polarity");
                measure(ctx, hardware, scale).await
            }
        })
        .await?;
    ctx.record(|characterisation| {
        characterisation.cold_reverse_current_amps = Some(current_measurement);
        characterisation.cold_reverse_voltage_volts = Some(voltage);
    });
    ctx.save();

    ramp_to(&ctx, &hardware, 0.0).await?;
    filament.set_output_enabled(false).await?;
    ctx.text("Disabled the output");

    Ok(())
}

/// Waits for the filament to settle at the present current and polarity, then
/// samples its voltage and current, returning them in that order.
async fn measure(
    ctx: Context,
    hardware: Hardware,
    scale: f64,
) -> Result<(Measurement, Measurement), ProcedureError> {
    match wait_for_settle(&ctx, &hardware, scale).await? {
        Settle::Settled(time) => {
            ctx.measurement("Settle time", format!("{:.1} s", time.as_secs_f64()));
        }
        Settle::TimedOut(time) => {
            warn!(
                "the filament didn't settle within {:.0} s",
                time.as_secs_f64()
            );
            ctx.text(format!(
                "Warning: the filament didn't settle within {:.0} s, so it's being measured anyway",
                time.as_secs_f64()
            ));
        }
    }

    let (voltages, currents) = take_samples(&ctx, &hardware, scale, SAMPLE_COUNT).await?;

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
    filament.set_heating_current_limit(0.0).await?;
    filament.set_output_enabled(false).await?;
    filament.set_polarity(Polarity::Nil).await?;
    ctx.text("Zeroed the current, disabled the output and de-energised the relays");

    ctx.save();
    Ok(())
}

// Helpers.

/// Changes the direction of the current through the filament, leaving the
/// output enabled at the unchanged setpoint.
///
/// The output is disabled while the relays move, since switching them under
/// load would arc their contacts. There's deliberately no ramp down and back
/// up: the filament dissipates the same power in either direction, and the
/// jump is harmless at these currents. Ramping would only lengthen the time it
/// spends cooling, and so the settle that follows.
async fn change_polarity(hardware: &Hardware, polarity: Polarity) -> Result<(), ProcedureError> {
    let filament = &hardware.filament;
    filament.set_output_enabled(false).await?;
    filament.set_polarity(polarity).await?;
    tokio::time::sleep(RELAY_SETTLE_TIME).await;
    filament.set_output_enabled(true).await?;
    Ok(())
}

/// Fails with `ProcedureError::Check` if the chamber isn't fit for a powered
/// filament: the pressure has risen above `FILAMENT_ABORT_PRESSURE_MBAR`, or
/// the TMP has stopped.
///
/// Called between every sample and every ramp step.
fn check_conditions(ctx: &Context) -> Result<(), ProcedureError> {
    let vacuum = ctx.snapshot().vacuum;

    if vacuum.pressure.value > FILAMENT_ABORT_PRESSURE_MBAR {
        error!(
            "the pressure rose to {:.1e} mbar with the filament powered",
            vacuum.pressure.value
        );
        return Err(ProcedureError::Check(format!(
            "the pressure rose to {:.1e} mbar, above {:.0e} mbar",
            vacuum.pressure.value, FILAMENT_ABORT_PRESSURE_MBAR
        )));
    }

    if !vacuum.tmp_running {
        error!("the TMP stopped with the filament powered");
        return Err(ProcedureError::Check(String::from("the TMP stopped")));
    }

    Ok(())
}

/// The mean and standard error of `values`, or `None` if there are fewer than
/// two.
fn mean_and_standard_error(values: &[f64]) -> Option<(f64, f64)> {
    if values.len() < 2 {
        return None;
    }

    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0);
    Some((mean, (variance / n).sqrt()))
}

/// Ramps the current from its present setpoint to `to` in steps of
/// `RAMP_STEP_AMPS`, one every `RAMP_STEP_INTERVAL`, checking the chamber
/// before each.
///
/// Starts from the setpoint rather than the measured current, which can be off
/// by as much as a step.
async fn ramp_to(ctx: &Context, hardware: &Hardware, to: f64) -> Result<(), ProcedureError> {
    ctx.waiting(format!("Ramping to {:.3} A", to), || async {
        let from = hardware.filament.get_heating_current_limit().await?;
        for current in ramp_steps(from, to) {
            check_conditions(ctx)?;
            hardware.filament.set_heating_current_limit(current).await?;
            tokio::time::sleep(RAMP_STEP_INTERVAL).await;
        }
        Ok(())
    })
    .await
}

/// The currents `ramp_to` sets on its way from `from` to `to`: steps of
/// `RAMP_STEP_AMPS`, finishing exactly on `to`. Empty if they're equal.
fn ramp_steps(from: f64, to: f64) -> Vec<f64> {
    // Less a little, so rounding can't add a step: 0.1 / 0.005 is
    // 20.000000000000004.
    let steps = ((to - from).abs() / RAMP_STEP_AMPS - 1e-9).ceil().max(0.0) as usize;
    let direction = (to - from).signum();

    let mut currents: Vec<f64> = (1..steps)
        .map(|k| from + direction * k as f64 * RAMP_STEP_AMPS)
        .collect();
    if steps > 0 {
        currents.push(to);
    }
    currents
}

/// Takes one sample of the filament's voltage (the acquisition's average) and
/// current, in that order, failing with `ProcedureError::Check` if anything
/// is wrong.
///
/// Checks the chamber first, and after the readings, that overcurrent
/// protection hasn't tripped, that the supply is still in constant current, and
/// that the voltage isn't clipped on `scale`.
async fn sample(
    ctx: &Context,
    hardware: &Hardware,
    scale: f64,
) -> Result<(f64, f64), ProcedureError> {
    let filament = &hardware.filament;

    check_conditions(ctx)?;

    let voltages = filament.get_filament_voltages().await?;
    let current = filament.get_heating_current().await?;

    // Before the mode, because a tripped output isn't in constant current
    // either, and this is the more useful message.
    if filament.get_overcurrent_tripped().await? {
        error!("overcurrent protection tripped");
        return Err(ProcedureError::Check(String::from(
            "overcurrent protection tripped",
        )));
    }

    let mode = filament.get_regulation_mode().await?;
    if mode != RegulationMode::ConstantCurrent {
        error!("the supply left constant current: {:?}", mode);
        return Err(ProcedureError::Check(format!(
            "the supply left constant current ({:?}), so the filament circuit is probably open",
            mode
        )));
    }

    let Some(voltage) = unclipped_average(voltages, scale) else {
        error!("the filament voltage clipped: {:?}", voltages);
        return Err(ProcedureError::Check(String::from(
            "the filament voltage clipped; the scale must not change mid-run, so restart the run",
        )));
    };

    Ok((voltage, current))
}

/// Takes `n` samples of the filament's voltage and current, returning them in
/// that order.
async fn take_samples(
    ctx: &Context,
    hardware: &Hardware,
    scale: f64,
    n: usize,
) -> Result<(Vec<f64>, Vec<f64>), ProcedureError> {
    ctx.waiting(format!("Taking {} samples", n), || async {
        let mut voltages = Vec::with_capacity(n);
        let mut currents = Vec::with_capacity(n);
        for _ in 0..n {
            let (voltage, current) = sample(ctx, hardware, scale).await?;
            voltages.push(voltage);
            currents.push(current);
        }
        Ok((voltages, currents))
    })
    .await
}

/// The average of `voltages`, or `None` if it's clipped on `scale`: if any
/// value is invalid, or the peak is beyond `CLIPPING_HEADROOM` of the way to
/// the edge of the screen.
fn unclipped_average(voltages: Voltages, scale: f64) -> Option<f64> {
    let limit = CLIPPING_HEADROOM * 4.0 * scale;
    match voltages {
        Voltages {
            average: Some(average),
            maximum: Some(maximum),
            minimum: Some(minimum),
        } if maximum.abs().max(minimum.abs()) <= limit => Some(average),
        _ => None,
    }
}

/// Samples until the filament's resistance stops changing, or until
/// `MAXIMUM_SETTLE_TIME`, without recording the samples.
///
/// Compares the latest `SETTLE_WINDOW` of quick resistances ($V/I$ in one
/// polarity) with the window before it. Offsets and thermal EMFs make the quick
/// resistance slightly wrong, but they're constant over seconds, so they
/// cancel in the comparison.
async fn wait_for_settle(
    ctx: &Context,
    hardware: &Hardware,
    scale: f64,
) -> Result<Settle, ProcedureError> {
    ctx.waiting("Waiting for the filament to settle", || async {
        let start = Instant::now();
        let mut resistances = Vec::new();

        loop {
            let (voltage, current) = sample(ctx, hardware, scale).await?;
            let now = Instant::now();
            resistances.push((now, voltage / current));

            let elapsed = now - start;
            if elapsed >= 2 * SETTLE_WINDOW && is_settled(&resistances, now) {
                return Ok(Settle::Settled(elapsed));
            }
            if elapsed >= MAXIMUM_SETTLE_TIME {
                return Ok(Settle::TimedOut(elapsed));
            }
        }
    })
    .await
}

/// Whether the quick resistances in the latest `SETTLE_WINDOW` before `now`
/// (B) differ from those in the window before it (A) by less than
/// `SETTLE_TOLERANCE`, or by less than noise can explain.
///
/// This is control flow rather than a reported statistic, so it's done here
/// rather than in Python.
fn is_settled(resistances: &[(Instant, f64)], now: Instant) -> bool {
    let window = |newest: Duration, oldest: Duration| -> Vec<f64> {
        resistances
            .iter()
            .filter(|(time, _)| {
                let age = now - *time;
                age >= newest && age < oldest
            })
            .map(|(_, resistance)| *resistance)
            .collect()
    };

    let (Some((mean_a, error_a)), Some((mean_b, error_b))) = (
        mean_and_standard_error(&window(SETTLE_WINDOW, 2 * SETTLE_WINDOW)),
        mean_and_standard_error(&window(Duration::ZERO, SETTLE_WINDOW)),
    ) else {
        return false;
    };

    // The difference is either negligible or undetectable. On its own, the
    // noise term would demand arbitrary flatness when the noise is low, and
    // the tolerance could be exceeded by chance alone when it's high. The
    // quick resistance is negative in the reverse polarity, hence `abs`.
    let difference_uncertainty = (error_a.powi(2) + error_b.powi(2)).sqrt();
    (mean_b - mean_a).abs() < (SETTLE_TOLERANCE * mean_b.abs()).max(2.0 * difference_uncertainty)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::{MockFilamentSystem, MockVacuumSystem};
    use std::sync::Arc;

    #[test]
    fn ramp_steps_end_exactly_on_the_target() {
        let up = ramp_steps(0.0, 0.1);
        assert_eq!(up.len(), 20);
        assert_eq!(up.last(), Some(&0.1));
        assert!((up[0] - 0.005).abs() < 1e-12);

        let down = ramp_steps(0.3, 0.1);
        assert_eq!(down.len(), 40);
        assert_eq!(down.last(), Some(&0.1));
        assert!((down[0] - 0.295).abs() < 1e-12);

        // A distance that isn't a whole number of steps ends with a short one.
        let uneven = ramp_steps(0.0, 0.012);
        assert_eq!(uneven.len(), 3);
        assert!((uneven[1] - 0.010).abs() < 1e-12);
        assert_eq!(uneven[2], 0.012);

        assert!(ramp_steps(0.1, 0.1).is_empty());
    }

    #[test]
    fn ramp_steps_are_never_larger_than_a_step() {
        let steps = ramp_steps(0.0, 0.3);
        let mut previous = 0.0;
        for current in steps {
            assert!(current - previous <= RAMP_STEP_AMPS + 1e-12);
            previous = current;
        }
    }

    // The mock refuses to switch the relays with the output enabled, so this
    // also checks the output is disabled first.
    #[tokio::test]
    async fn change_polarity_leaves_the_output_on_at_the_same_setpoint() {
        let hardware = Hardware {
            filament: Arc::new(MockFilamentSystem::default()),
            vacuum: Arc::new(MockVacuumSystem::default()),
        };
        let filament = &hardware.filament;
        filament.set_polarity(Polarity::Forward).await.unwrap();
        filament.set_heating_current_limit(0.1).await.unwrap();
        filament.set_output_enabled(true).await.unwrap();

        change_polarity(&hardware, Polarity::Reverse).await.unwrap();

        let snapshot = filament.snapshot().await.unwrap();
        assert_eq!(snapshot.polarity, Polarity::Reverse);
        assert!(snapshot.output_enabled);
        assert_eq!(snapshot.heating_current, 0.1);
    }
}
