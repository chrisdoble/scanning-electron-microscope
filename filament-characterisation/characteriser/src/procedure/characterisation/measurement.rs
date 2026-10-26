//! What every measurement in the procedure uses: ramping, changing the
//! polarity, sampling, settling and choosing a vertical scale, with the
//! constants they use. The parts that don't need a `Context` are in
//! `sampling`, so binaries without the terminal UI can use them too.

use crate::{
    constants::FILAMENT_ABORT_PRESSURE_MBAR,
    hardware::Hardware,
    procedure::{Context, ProcedureError},
    python,
    results::Measurement,
    sampling::{
        RELAY_SETTLE_TIME, VERTICAL_SCALES_VOLTS_PER_DIVISION, mean_and_standard_error,
        unclipped_average,
    },
};
use host::power_supply::{Polarity, RegulationMode};
use log::*;
use std::time::Duration;
use tokio::time::Instant;

/// The supply's current readback resolution in amperes, with the HIRES option.
pub(super) const CURRENT_READBACK_RESOLUTION_AMPS: f64 = 1e-4;

/// How long `wait_for_settle` waits before giving up and carrying on.
const MAXIMUM_SETTLE_TIME: Duration = Duration::from_secs(60);

/// How far `ramp_to` moves the current at each step in amperes.
///
/// With `RAMP_STEP_INTERVAL`, 50 mA/s. At cold-resistance currents ramping has
/// no physical benefit; it's there so `ramp_to` is ready for hot measurements,
/// where each jump should be small compared with what the filament's thermal
/// time constant smooths out.
const RAMP_STEP_AMPS: f64 = 0.005;

/// How long `ramp_to` waits between steps.
const RAMP_STEP_INTERVAL: Duration = Duration::from_millis(100);

/// How long to wait after changing the setpoint, or re-enabling the output,
/// before a single reading is taken as being at the new current.
///
/// The supply has been seen to take about 0.6 s to apply a setpoint change
/// (see "Appendix: the supply's current readback" in
/// `filament-characterisation/docs/2_COLD_RESISTANCE.md`).
/// `wait_for_settle` doesn't need this, because it compares seconds of
/// readings.
pub(super) const SETPOINT_APPLY_TIME: Duration = Duration::from_secs(1);

/// How many samples each measured quantity is summarised from.
pub(super) const SAMPLE_COUNT: usize = 20;

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

/// How `wait_for_settle` finished, and after how long.
pub(super) enum Settle {
    Settled(Duration),
    TimedOut(Duration),
}

/// Changes the direction of the current through the filament, leaving the
/// output enabled at the unchanged setpoint.
///
/// The output is disabled while the relays move, since switching them under
/// load would arc their contacts. There's deliberately no ramp down and back
/// up: the filament dissipates the same power in either direction, and the
/// jump is harmless at these currents. Ramping would only lengthen the time it
/// spends cooling, and so the settle that follows.
pub(super) async fn change_polarity(
    hardware: &Hardware,
    polarity: Polarity,
) -> Result<(), ProcedureError> {
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
/// Called between every sample and every ramp step. Checks nothing without
/// vacuum (`--no-vacuum`), when the chamber is at atmospheric pressure and the
/// TMP is off by design.
fn check_conditions(ctx: &Context) -> Result<(), ProcedureError> {
    if !ctx.in_vacuum() {
        return Ok(());
    }

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

/// Ramps the current from its present setpoint to `to` in steps of
/// `RAMP_STEP_AMPS`, one every `RAMP_STEP_INTERVAL`, checking the chamber
/// before each.
///
/// Starts from the setpoint rather than the measured current, which can be off
/// by as much as a step.
pub(super) async fn ramp_to(
    ctx: &Context,
    hardware: &Hardware,
    to: f64,
) -> Result<(), ProcedureError> {
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

/// Reads the filament voltage's average, stepping the vertical scale up through
/// `VERTICAL_SCALES_VOLTS_PER_DIVISION` while it clips. `scale_index` is the
/// scale in use, and is left at the one that didn't clip.
///
/// Checks the chamber before each reading, as `sample` does.
pub(super) async fn read_unclipped(
    ctx: &Context,
    hardware: &Hardware,
    scale_index: &mut usize,
) -> Result<f64, ProcedureError> {
    let filament = &hardware.filament;
    loop {
        check_conditions(ctx)?;
        let scale = VERTICAL_SCALES_VOLTS_PER_DIVISION[*scale_index];
        let voltages = filament.get_filament_voltages().await?;
        if let Some(average) = unclipped_average(voltages, scale) {
            return Ok(average);
        }

        *scale_index += 1;
        let Some(&next) = VERTICAL_SCALES_VOLTS_PER_DIVISION.get(*scale_index) else {
            error!("the filament voltage clips even at {} V/div", scale);
            return Err(ProcedureError::Check(format!(
                "the filament voltage clips even at {} V/div",
                scale
            )));
        };
        info!(
            "the filament voltage clipped, so increasing the scale to {} V/div",
            next
        );
        filament.set_vertical_scale(next).await?;
    }
}

/// Takes one sample of the filament's voltage (the acquisition's average) and
/// current, in that order, failing with `ProcedureError::Check` if anything
/// is wrong, or `ProcedureError::Clipped` if the voltage clipped.
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
        // Not necessarily an error: the hot sweep steps the scale up and
        // measures again.
        info!("the filament voltage clipped: {:?}", voltages);
        return Err(ProcedureError::Clipped(format!(
            "the filament voltage clipped at {} V/div",
            scale
        )));
    };

    Ok((voltage, current))
}

/// Takes `n` samples of the filament's voltage and current, returning them in
/// that order, followed by the midpoint of the sampling window in seconds since
/// `start`.
pub(super) async fn take_samples(
    ctx: &Context,
    hardware: &Hardware,
    scale: f64,
    n: usize,
    start: Instant,
) -> Result<(Vec<f64>, Vec<f64>, f64), ProcedureError> {
    ctx.waiting(format!("Taking {} samples", n), || async {
        let window_start = Instant::now();
        let mut voltages = Vec::with_capacity(n);
        let mut currents = Vec::with_capacity(n);
        for _ in 0..n {
            let (voltage, current) = sample(ctx, hardware, scale).await?;
            voltages.push(voltage);
            currents.push(current);
        }
        let midpoint = window_start + (Instant::now() - window_start) / 2;
        Ok((voltages, currents, (midpoint - start).as_secs_f64()))
    })
    .await
}

/// Takes `n` readings of the filament's voltage on `scale` with no current
/// flowing, e.g. with the output off, for the scope's offset.
///
/// Unlike `take_samples`, it doesn't read the current or check the supply,
/// which isn't regulating. It still checks the chamber before each reading. A
/// reading that clips with no current means something's wrong, so it fails with
/// `ProcedureError::Check`.
pub(super) async fn take_offset_samples(
    ctx: &Context,
    hardware: &Hardware,
    scale: f64,
    n: usize,
) -> Result<Vec<f64>, ProcedureError> {
    ctx.waiting(format!("Taking {} samples", n), || async {
        let mut voltages = Vec::with_capacity(n);
        for _ in 0..n {
            check_conditions(ctx)?;
            let readings = hardware.filament.get_filament_voltages().await?;
            let Some(voltage) = unclipped_average(readings, scale) else {
                error!(
                    "the voltage clipped with no current on {} V/div: {:?}",
                    scale, readings
                );
                return Err(ProcedureError::Check(format!(
                    "the voltage clipped with no current on {} V/div",
                    scale
                )));
            };
            voltages.push(voltage);
        }
        Ok(voltages)
    })
    .await
}

/// Samples until the filament's resistance stops changing, or until
/// `MAXIMUM_SETTLE_TIME`, without recording the samples.
///
/// Compares the latest `SETTLE_WINDOW` of quick resistances ($V/I$ in one
/// polarity) with the window before it. Offsets and thermal EMFs make the quick
/// resistance slightly wrong, but they're constant over seconds, so they
/// cancel in the comparison.
pub(super) async fn wait_for_settle(
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
pub(super) async fn summarise(samples: Vec<f64>) -> Result<Measurement, ProcedureError> {
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
