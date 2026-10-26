//! Measuring the hot sweep: the filament's resistance at setpoints up to a
//! maximum current and back down again. See
//! `filament-characterisation/docs/5_HOT_SWEEP.md`.

use super::measurement::{
    CURRENT_READBACK_RESOLUTION_AMPS, SAMPLE_COUNT, Settle, ramp_to, read_unclipped, summarise,
    take_offset_samples, take_samples, wait_for_settle,
};
use crate::{
    hardware::Hardware,
    procedure::{Context, ProcedureError},
    python::{self, HotSweepPointInput},
    results::{HotSweep, HotSweepDirection, HotSweepOffset, HotSweepPoint, Measurement},
    sampling::{
        OVERCURRENT_PROTECTION_MARGIN_AMPS, SETPOINT_APPLY_TIME, VERTICAL_SCALES_VOLTS_PER_DIVISION,
    },
};
use host::power_supply::Polarity;
use log::*;
use tokio::time::Instant;

/// The sweep's maximum current in amperes.
///
/// The filament's epoxy mount softens at about 80 °C, so this starts low. Raise
/// it only once the previous maximum's change between the way up and the way
/// down, at the lowest setpoints, shows the mount warmed acceptably.
const HOT_SWEEP_MAXIMUM_CURRENT_AMPS: f64 = 0.6;

/// The voltage limit while measuring the hot sweep in volts.
///
/// A hot filament at 3 A, with its leads and relays, needs a few volts. A low
/// limit keeps the channel in constant current while limiting the energy
/// delivered if the circuit opens and re-closes.
const HOT_SWEEP_VOLTAGE_LIMIT_VOLTS: f64 = 5.0;

/// A setpoint's voltage and current samples, and how they were taken.
struct Sampled {
    currents: Vec<f64>,

    /// How long the filament took to settle, in seconds.
    settle_seconds: f64,

    /// The midpoint of the sampling window, in seconds since the sweep started.
    time_seconds: f64,

    voltages: Vec<f64>,

    /// Set if the filament didn't settle in time.
    warning: Option<String>,
}

/// Measures the filament's resistance at setpoints up to
/// `HOT_SWEEP_MAXIMUM_CURRENT_AMPS` and back down, in one polarity, after
/// reading the scope's offset on every vertical scale.
pub(super) async fn measure_hot_sweep(
    ctx: Context,
    hardware: Hardware,
) -> Result<(), ProcedureError> {
    let filament = &hardware.filament;

    // Every point's time is relative to this.
    let start = Instant::now();

    // Setup. Don't assume the output is off: it's however the supply was left.
    let overcurrent_protection =
        HOT_SWEEP_MAXIMUM_CURRENT_AMPS + OVERCURRENT_PROTECTION_MARGIN_AMPS;
    filament.set_output_enabled(false).await?;
    filament.set_heating_current_limit(0.0).await?;
    filament
        .set_heating_voltage_limit(HOT_SWEEP_VOLTAGE_LIMIT_VOLTS)
        .await?;
    filament
        .set_overcurrent_protection(overcurrent_protection)
        .await?;
    ctx.text(format!(
        "Set the voltage limit to {:.1} V and overcurrent protection to {:.3} A",
        HOT_SWEEP_VOLTAGE_LIMIT_VOLTS, overcurrent_protection
    ));

    // No reversal: at these voltages, what it would cancel is negligible apart
    // from the scope's offset, which is read and subtracted instead.
    filament.set_polarity(Polarity::Forward).await?;
    ctx.text("Set the polarity to Forward");

    let (step, setpoints) = hot_sweep_setpoints(HOT_SWEEP_MAXIMUM_CURRENT_AMPS);
    ctx.measurement(
        "Setpoints",
        format!(
            "{} up to {:.0} mA and {} back down, {} A² apart",
            setpoints.len(),
            HOT_SWEEP_MAXIMUM_CURRENT_AMPS * 1000.0,
            setpoints.len() - 1,
            step
        ),
    );
    ctx.record(|characterisation| {
        characterisation.hot_sweep = Some(HotSweep {
            maximum_current_amps: HOT_SWEEP_MAXIMUM_CURRENT_AMPS,
            offsets: Vec::new(),
            points: Vec::new(),
            step_amps_squared: step,
        })
    });
    ctx.save();

    let offsets = ctx
        .section("Reading the scope's offsets", |ctx| {
            read_offsets(ctx, hardware.clone())
        })
        .await?;

    filament.set_output_enabled(true).await?;
    ctx.text("Enabled the output");

    // Up through every setpoint, then down through all but the top, which
    // isn't measured twice. Numbered together, so the count shows how far
    // through the whole sweep is.
    let plan: Vec<(HotSweepDirection, usize)> = (0..setpoints.len())
        .map(|index| (HotSweepDirection::Up, index))
        .chain(
            (0..setpoints.len() - 1)
                .rev()
                .map(|index| (HotSweepDirection::Down, index)),
        )
        .collect();
    let total = plan.len();

    // Each setpoint's scale index and resistance going up, for the way down.
    let mut ups: Vec<(usize, f64)> = Vec::with_capacity(setpoints.len());
    let mut scale_index = 0;
    for (index, (direction, setpoint_index)) in plan.into_iter().enumerate() {
        let setpoint = setpoints[setpoint_index];
        let up = match direction {
            HotSweepDirection::Up => None,
            HotSweepDirection::Down => Some(ups[setpoint_index]),
        };
        let title = format!(
            "Measuring at {:.0} mA ({}, {}/{})",
            setpoint * 1000.0,
            match direction {
                HotSweepDirection::Up => "up",
                HotSweepDirection::Down => "down",
            },
            index + 1,
            total
        );

        // Going down, each setpoint starts on the scale it was measured on
        // going up, so the scope's gain error cancels in their difference.
        let starting_scale = up.map_or(scale_index, |(up_scale, _)| up_scale);
        let offsets = &offsets;
        let (measured_scale, resistance) = ctx
            .section(title, |ctx| {
                measure_point(
                    ctx,
                    hardware.clone(),
                    setpoint,
                    starting_scale,
                    up.map(|(_, up_resistance)| up_resistance),
                    offsets,
                    start,
                )
            })
            .await?;

        if direction == HotSweepDirection::Up {
            scale_index = measured_scale;
            ups.push((measured_scale, resistance));
        }
    }

    ramp_to(&ctx, &hardware, 0.0).await?;
    filament.set_output_enabled(false).await?;
    ctx.text("Disabled the output");
    ctx.save();
    Ok(())
}

/// Reads the scope with no current on every vertical scale, recording each
/// reading as that scale's offset, and returns them in the order of
/// `VERTICAL_SCALES_VOLTS_PER_DIVISION`. The output must be off.
///
/// The reading is everything in the voltage path with no current flowing: the
/// scope's offset error, which grows with the scale, net thermal EMFs, and any
/// DC pickup. Without reversal, it's subtracted instead.
async fn read_offsets(
    ctx: Context,
    hardware: Hardware,
) -> Result<Vec<Measurement>, ProcedureError> {
    let mut offsets = Vec::with_capacity(VERTICAL_SCALES_VOLTS_PER_DIVISION.len());
    for scale in VERTICAL_SCALES_VOLTS_PER_DIVISION {
        hardware.filament.set_vertical_scale(scale).await?;
        let samples = take_offset_samples(&ctx, &hardware, scale, SAMPLE_COUNT).await?;
        let offset = summarise(samples).await?;
        ctx.measurement(
            format!("Offset at {}", scale_label(scale)),
            format!(
                "{:+.3} ± {:.3} mV",
                offset.value * 1000.0,
                offset.uncertainty * 1000.0
            ),
        );

        record_hot_sweep(&ctx, |hot_sweep| {
            hot_sweep.offsets.push(HotSweepOffset {
                vertical_scale_volts_per_division: scale,
                voltage_volts: offset.clone(),
            })
        });
        ctx.save();
        offsets.push(offset);
    }
    Ok(offsets)
}

/// Measures the filament at `setpoint`, analyses it, and records it, starting
/// on the scale at `scale_index` in `VERTICAL_SCALES_VOLTS_PER_DIVISION`.
/// Returns the index of the scale it was measured on, and its resistance in
/// ohms.
///
/// `up` is `None` going up, and the resistance measured at this setpoint going
/// up when going down. `offsets` are the scope's offsets, by scale. Times are
/// recorded relative to `start`.
async fn measure_point(
    ctx: Context,
    hardware: Hardware,
    setpoint: f64,
    mut scale_index: usize,
    up: Option<f64>,
    offsets: &[Measurement],
    start: Instant,
) -> Result<(usize, f64), ProcedureError> {
    let filament = &hardware.filament;
    let direction = match up {
        None => HotSweepDirection::Up,
        Some(_) => HotSweepDirection::Down,
    };
    let starting_scale = scale_index;

    ramp_to(&ctx, &hardware, setpoint).await?;
    filament
        .set_vertical_scale(VERTICAL_SCALES_VOLTS_PER_DIVISION[scale_index])
        .await?;

    // Going up, the voltage only rises, so the scale may need to. A reading
    // once the supply has applied the setpoint chooses it, and
    // `CLIPPING_HEADROOM` leaves room for the rise as the filament settles.
    if direction == HotSweepDirection::Up {
        ctx.waiting("Choosing the vertical scale", || async {
            tokio::time::sleep(SETPOINT_APPLY_TIME).await;
            read_unclipped(&ctx, &hardware, &mut scale_index).await
        })
        .await?;
    }

    // A reading that clips anyway steps the scale up and measures the point
    // again, keeping only the measurement on the last scale.
    let sampled = loop {
        let scale = VERTICAL_SCALES_VOLTS_PER_DIVISION[scale_index];
        match settle_and_sample(&ctx, &hardware, scale, start).await {
            Ok(sampled) => break sampled,
            Err(ProcedureError::Clipped(_)) => {
                scale_index += 1;
                let Some(&next) = VERTICAL_SCALES_VOLTS_PER_DIVISION.get(scale_index) else {
                    error!("the filament voltage clips even at {} V/div", scale);
                    return Err(ProcedureError::Check(format!(
                        "the filament voltage clips even at {} V/div",
                        scale
                    )));
                };
                filament.set_vertical_scale(next).await?;
                ctx.text(format!(
                    "The voltage clipped, so measuring again at {}",
                    scale_label(next)
                ));
            }
            Err(e) => return Err(e),
        }
    };

    let scale = VERTICAL_SCALES_VOLTS_PER_DIVISION[scale_index];
    let mut warnings: Vec<String> = sampled.warning.into_iter().collect();
    if direction == HotSweepDirection::Down && scale_index != starting_scale {
        let warning = format!(
            "the voltage clipped at {}, the scale used going up, so this point was measured at {} and its change since the way up includes both scales' gain errors",
            scale_label(VERTICAL_SCALES_VOLTS_PER_DIVISION[starting_scale]),
            scale_label(scale)
        );
        warn!("{}", warning);
        ctx.text(format!("Warning: {}", warning));
        warnings.push(warning);
    }

    let point = HotSweepPoint {
        analysis: None,
        current_amps: summarise(sampled.currents).await?,
        direction,
        settle_seconds: sampled.settle_seconds,
        setpoint_amps: setpoint,
        time_seconds: sampled.time_seconds,
        vertical_scale_volts_per_division: scale,
        voltage_volts: summarise(sampled.voltages).await?,
        warnings,
    };
    let input = HotSweepPointInput {
        current_amps: point.current_amps.clone(),
        current_resolution_amps: CURRENT_READBACK_RESOLUTION_AMPS,
        offset_voltage_volts: offsets[scale_index].clone(),
        voltage_volts: point.voltage_volts.clone(),
    };

    // Saved before it's analysed, so the measurements survive an analysis
    // that fails.
    record_hot_sweep(&ctx, |hot_sweep| hot_sweep.points.push(point));
    ctx.save();

    let analysis = python::hot_sweep_point(&input).await?;
    let resistance = analysis.resistance_ohms;
    ctx.measurement(
        "Resistance",
        format!(
            "{:.2} ± {:.2} mΩ",
            resistance.value * 1000.0,
            resistance.uncertainty * 1000.0
        ),
    );
    ctx.measurement(
        "Power",
        format!(
            "{:.1} mW",
            analysis.voltage_volts.value * analysis.current_amps.value * 1000.0
        ),
    );

    // The number the operator watches: going down, at the lowest setpoints,
    // it approximates how much the mount warmed. Only a difference, so it's
    // fine in Rust.
    if let Some(up) = up {
        let change = resistance.value - up;
        ctx.measurement(
            "Change since the way up",
            format!("{:+.2} mΩ ({:+.2} %)", change * 1000.0, change / up * 100.0),
        );
    }

    record_hot_sweep(&ctx, |hot_sweep| {
        if let Some(point) = hot_sweep.points.last_mut() {
            point.analysis = Some(analysis);
        }
    });
    ctx.save();

    Ok((scale_index, resistance.value))
}

/// Waits for the filament to settle at the present current on `scale`, then
/// samples its voltage and current. Times are relative to `start`.
///
/// Fails with `ProcedureError::Clipped` if a reading clips, so the caller can
/// step the scale up.
async fn settle_and_sample(
    ctx: &Context,
    hardware: &Hardware,
    scale: f64,
    start: Instant,
) -> Result<Sampled, ProcedureError> {
    let (settle_time, warning) = match wait_for_settle(ctx, hardware, scale).await? {
        Settle::Settled(time) => (time, None),
        Settle::TimedOut(time) => {
            let warning = format!(
                "the filament didn't settle within {:.0} s, so it was measured anyway",
                time.as_secs_f64()
            );
            warn!("{}", warning);
            ctx.text(format!("Warning: {}", warning));
            (time, Some(warning))
        }
    };

    let (voltages, currents, time_seconds) =
        take_samples(ctx, hardware, scale, SAMPLE_COUNT, start).await?;
    Ok(Sampled {
        currents,
        settle_seconds: settle_time.as_secs_f64(),
        time_seconds,
        voltages,
        warning,
    })
}

/// The step in I², in A², and the currents to measure going up, in amperes:
/// 100 mA, then a grid evenly spaced in I² up to `maximum`, which must be above
/// 100 mA.
///
/// The grid is the multiples of the step, so runs to different maxima share
/// setpoints. The step is the smallest of `STEPS_AMPS_SQUARED` that keeps the
/// grid to at most `MAXIMUM_POINTS` points, so the sweep stays short however
/// high the maximum: a longer sweep would also warm the mount more. Above about
/// 2 A even the largest step gives more points than that, and it's used anyway.
/// Every step divides 0.2 A², so multiples of it are setpoints at any maximum.
///
/// 100 mA comes first whatever the step, because it's where the filament's own
/// heating is smallest, so its up/down difference best shows how much the mount
/// warmed. The maximum is always the last point, added if it isn't on the grid.
fn hot_sweep_setpoints(maximum: f64) -> (f64, Vec<f64>) {
    const STEPS_AMPS_SQUARED: [f64; 5] = [0.01, 0.02, 0.05, 0.1, 0.2];
    const MAXIMUM_POINTS: usize = 20;
    const LOWEST_AMPS: f64 = 0.1;

    // In A². Floating point puts some multiples a hair off: 0.6² / 0.02 is
    // 17.999… in f64, but 0.36 A² is on the grid.
    const TOLERANCE: f64 = 1e-9;

    let maximum_squared = maximum * maximum;

    // The grid for `step`: its multiples above 100 mA, up to the maximum.
    let grid = |step: f64| -> Vec<f64> {
        let last = (maximum_squared / step + TOLERANCE).floor() as usize;
        let mut currents: Vec<f64> = (1..=last)
            .map(|k| k as f64 * step)
            .filter(|&squared| squared > LOWEST_AMPS * LOWEST_AMPS + TOLERANCE)
            .map(f64::sqrt)
            .collect();

        // Exactly the maximum, rather than a square root that rounds near it.
        if (last as f64 * step - maximum_squared).abs() < TOLERANCE {
            if let Some(top) = currents.last_mut() {
                *top = maximum;
            }
        } else {
            currents.push(maximum);
        }
        currents
    };

    let step = STEPS_AMPS_SQUARED
        .into_iter()
        .find(|&step| grid(step).len() <= MAXIMUM_POINTS)
        .unwrap_or(STEPS_AMPS_SQUARED[STEPS_AMPS_SQUARED.len() - 1]);

    let mut setpoints = vec![LOWEST_AMPS];
    setpoints.extend(grid(step));
    (step, setpoints)
}

/// Records into the hot sweep's results.
fn record_hot_sweep(ctx: &Context, f: impl FnOnce(&mut HotSweep)) {
    ctx.record(|characterisation| {
        f(characterisation
            .hot_sweep
            .as_mut()
            .expect("the hot sweep results were created during setup"))
    });
}

/// A vertical scale for display, e.g. "20 mV/div" or "2 V/div".
fn scale_label(scale: f64) -> String {
    if scale < 1.0 {
        format!("{:.0} mV/div", scale * 1000.0)
    } else {
        format!("{} V/div", scale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether `a` and `b` are equal to within floating-point rounding.
    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn hot_sweep_setpoints_to_0_6_amps() {
        let (step, setpoints) = hot_sweep_setpoints(0.6);
        assert_eq!(step, 0.02);
        assert_eq!(setpoints.len(), 19);
        assert_eq!(setpoints[0], 0.1);
        assert!(close(setpoints[1], 0.02_f64.sqrt()));
        assert_eq!(setpoints[18], 0.6);
    }

    #[test]
    fn hot_sweep_setpoints_to_1_amp() {
        let (step, setpoints) = hot_sweep_setpoints(1.0);
        assert_eq!(step, 0.05);
        assert_eq!(setpoints.len(), 21);
        assert_eq!(setpoints.last(), Some(&1.0));
    }

    #[test]
    fn hot_sweep_setpoints_to_1_6_amps() {
        let (step, setpoints) = hot_sweep_setpoints(1.6);
        assert_eq!(step, 0.2);
        assert_eq!(setpoints.len(), 14);
        assert!(close(setpoints[1], 0.2_f64.sqrt()));
        assert_eq!(setpoints.last(), Some(&1.6));
    }

    #[test]
    fn hot_sweep_setpoints_start_at_100_milliamps_once() {
        // With a 0.01 A² step, the grid's first multiple is 100 mA itself.
        let (step, setpoints) = hot_sweep_setpoints(0.4);
        assert_eq!(step, 0.01);
        assert_eq!(setpoints[0], 0.1);
        assert!(setpoints[1] > 0.1);
        assert_eq!(setpoints.len(), 16);
    }

    #[test]
    fn hot_sweep_setpoints_end_on_a_maximum_off_the_grid() {
        // 0.55² is 0.3025 A², between multiples of 0.02 A².
        let (step, setpoints) = hot_sweep_setpoints(0.55);
        assert_eq!(step, 0.02);
        assert_eq!(setpoints.last(), Some(&0.55));
        assert!(close(setpoints[setpoints.len() - 2], 0.30_f64.sqrt()));
    }

    #[test]
    fn hot_sweep_setpoints_increase_within_the_point_limit_and_nest() {
        // Every maximum in 10 mA steps from 110 mA to 2 A.
        for centiamps in 11..=200 {
            let maximum = f64::from(centiamps) / 100.0;
            let (step, setpoints) = hot_sweep_setpoints(maximum);

            assert!(
                setpoints.windows(2).all(|pair| pair[0] < pair[1]),
                "{} A: {:?}",
                maximum,
                setpoints
            );
            assert!(setpoints.len() - 1 <= 20, "{} A: {:?}", maximum, setpoints);
            assert_eq!(setpoints.last(), Some(&maximum));

            // Every multiple of 0.2 A² up to the maximum is a setpoint.
            let mut multiple = 0.2;
            while multiple <= maximum * maximum + 1e-9 {
                assert!(
                    setpoints
                        .iter()
                        .any(|&current| close(current * current, multiple)),
                    "{} A with a {} A² step is missing {} A²",
                    maximum,
                    step,
                    multiple
                );
                multiple += 0.2;
            }
        }
    }
}
