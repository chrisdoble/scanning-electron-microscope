//! The characterisation procedure itself.
//!
//! The top-level sections of the run, in order, each an ordinary async function
//! so the procedure reads as the sequence of things that happen. Every section
//! takes the `Hardware`: that's how one reads an instrument more often than the
//! once-a-second snapshot, as `take_samples` does.

use super::{Context, ProcedureError};
use crate::{
    constants::{
        FILAMENT_ABORT_PRESSURE_MBAR, FILAMENT_OPERATING_PRESSURE_MBAR,
        TMP_MAXIMUM_BACKING_PRESSURE_MBAR,
    },
    hardware::Hardware,
    python::{self, ColdResistanceFitInput, ColdResistanceFitPoint, ColdResistancePointInput},
    results::{ColdResistance, ColdResistanceFitParameters, ColdResistancePoint, Measurement},
};
use host::{
    oscilloscope::Voltages,
    power_supply::{Polarity, RegulationMode},
};
use log::*;
use rand::seq::SliceRandom;
use std::time::Duration;
use tokio::time::Instant;

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

/// The bound on the supply's current readback gain error, relative: the
/// percentage part of the DP900's readback accuracy, within 20–30 °C.
const CURRENT_GAIN_BOUND: f64 = 0.0015;

/// How much `CURRENT_GAIN_BOUND` widens for each degree outside 20–30 °C.
const CURRENT_GAIN_TEMPERATURE_COEFFICIENT_PER_CELSIUS: f64 = 1e-4;

/// The bound on the supply's current readback offset in amperes: the fixed
/// part of the DP900's readback accuracy, within 20–30 °C.
const CURRENT_OFFSET_BOUND_AMPS: f64 = 0.005;

/// How much `CURRENT_OFFSET_BOUND_AMPS` widens for each degree outside
/// 20–30 °C, in amperes.
const CURRENT_OFFSET_TEMPERATURE_COEFFICIENT_AMPS_PER_CELSIUS: f64 = 0.002;

/// The supply's current readback resolution in amperes, with the HIRES option.
const CURRENT_READBACK_RESOLUTION_AMPS: f64 = 1e-4;

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

/// How long to wait after changing the setpoint, or re-enabling the output,
/// before a single reading is taken as being at the new current.
///
/// The supply has been seen to take about 0.6 s to apply a setpoint change
/// (see "Appendix: the supply's current readback" in COLD_RESISTANCE.md).
/// `wait_for_settle` doesn't need this, because it compares seconds of
/// readings.
const SETPOINT_APPLY_TIME: Duration = Duration::from_secs(1);

/// The temperature the cold resistance is corrected to in °C.
const REFERENCE_TEMPERATURE_CELSIUS: f64 = 20.0;

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

/// The bound on the filament temperature in kelvin, before adding the drift
/// over the run: the thermometer's accuracy plus how far the filament might sit
/// from the flange's temperature.
///
/// A judgement. Tighten it if the thermometer and its placement justify it.
const THERMOMETER_BOUND_KELVIN: f64 = 1.0;

/// The bound on `TUNGSTEN_TEMPERATURE_COEFFICIENT_PER_KELVIN`.
///
/// A judgement: published values span roughly 0.0042–0.0048 /K depending on
/// purity and doping.
const TUNGSTEN_TEMPERATURE_COEFFICIENT_BOUND_PER_KELVIN: f64 = 0.0003;

/// Tungsten's temperature coefficient of resistance near room temperature,
/// $\alpha$.
const TUNGSTEN_TEMPERATURE_COEFFICIENT_PER_KELVIN: f64 = 0.0045;

/// The oscilloscope's vertical scales in volts per division, smallest first.
///
/// Starts at 10 mV/div, because the ±1% gain spec is only "typical" at 5 mV/div
/// and below.
const VERTICAL_SCALES_VOLTS_PER_DIVISION: [f64; 10] =
    [0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0];

/// The bound on the oscilloscope's gain error, relative.
///
/// The data sheet's ±1% of full scale is a bound on the absolute error at any
/// reading. Under the pure-gain model it bounds the gain at 2% read against a
/// single reading, the most that fits on screen, or 1% read against the
/// difference between a reading at the top of the screen and one at the
/// bottom. The second matches the measurement here, which takes half the
/// difference between two polarities, but the first is what the data sheet
/// literally guarantees (COLD_RESISTANCE.md, 3.4.3).
const VOLTAGE_GAIN_BOUND: f64 = 0.02;

/// The measurements in one polarity at one setpoint.
struct PolarityMeasurement {
    current: Measurement,
    settle_seconds: f64,
    voltage: Measurement,

    /// Set if the filament didn't settle in time.
    warning: Option<String>,
}

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

/// Measures the filament's resistance at each setpoint, in both directions.
///
/// Reversing the direction cancels everything that doesn't reverse with the
/// current, such as the Seebeck voltages at the filament's junctions and the
/// scope's offset.
async fn measure_cold_resistance(ctx: Context, hardware: Hardware) -> Result<(), ProcedureError> {
    let filament = &hardware.filament;

    // Setup. Don't assume the output is off: it's however the supply was left.
    let overcurrent_protection = largest_setpoint() + OVERCURRENT_PROTECTION_MARGIN_AMPS;
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

    // The unheated filament sits at about the temperature of its mount, which
    // is in contact with the flange, not the room air. Asked here rather than
    // in `prepare`, because the TMP warms the chamber during the pump-down.
    let chamber_start_temperature: f64 = ctx
        .input("Chamber temperature at the filament's flange", Some("°C"))
        .await?;

    // Only for the supply's accuracy band.
    let room_temperature: f64 = ctx
        .input("Room temperature near the power supply", Some("°C"))
        .await?;
    let (gain_bound, offset_bound) = current_readback_bounds(room_temperature);
    ctx.measurement(
        "Current readback gain bound",
        format!("{:.2} %", gain_bound * 100.0),
    );
    ctx.measurement(
        "Current readback offset bound",
        format!("{:.1} mA", offset_bound * 1000.0),
    );

    ctx.record(|characterisation| {
        characterisation.cold_resistance = Some(ColdResistance {
            analysis: None,
            chamber_end_temperature_celsius: None,
            chamber_start_temperature_celsius: chamber_start_temperature,
            fit_parameters: None,
            points: Vec::new(),
            positive_polarity: None,
            room_temperature_celsius: room_temperature,
            vertical_scale_volts_per_division: None,
        })
    });
    ctx.save();

    let (scale, positive_polarity) = ctx
        .section("Choosing the vertical scale", |ctx| {
            choose_vertical_scale(ctx, hardware.clone())
        })
        .await?;
    ctx.measurement("Vertical scale", format!("{} mV/div", scale * 1000.0));
    ctx.measurement("Positive polarity", positive_polarity);
    record_cold_resistance(&ctx, |cold_resistance| {
        cold_resistance.vertical_scale_volts_per_division = Some(scale);
        cold_resistance.positive_polarity = Some(positive_polarity);
    });
    ctx.save();

    // A random order stops slow drifts, such as the rig warming up, from
    // correlating with current.
    let mut setpoints = COLD_RESISTANCE_SETPOINTS_AMPS;
    setpoints.shuffle(&mut rand::rng());

    // Each setpoint starts in whichever polarity the relays are already in, and
    // ends in the other, so the order alternates. A slowly drifting voltage
    // that doesn't reverse biases a point by half the drift between its two
    // measurements, with a sign set by which came first: alternating makes
    // that cancel on average rather than bias every point the same way. The
    // scale search leaves the relays reversed.
    let mut polarity = Polarity::Reverse;
    for (index, setpoint) in setpoints.iter().copied().enumerate() {
        let title = format!(
            "Measuring at {:.0} mA ({}/{})",
            setpoint * 1000.0,
            index + 1,
            setpoints.len()
        );
        polarity = ctx
            .section(title, |ctx| {
                measure_setpoint(
                    ctx,
                    hardware.clone(),
                    setpoint,
                    polarity,
                    positive_polarity,
                    scale,
                )
            })
            .await?;
    }

    ramp_to(&ctx, &hardware, 0.0).await?;
    filament.set_output_enabled(false).await?;
    ctx.text("Disabled the output");

    let chamber_end_temperature: f64 = ctx
        .input("Chamber temperature at the filament's flange", Some("°C"))
        .await?;
    record_cold_resistance(&ctx, |cold_resistance| {
        cold_resistance.chamber_end_temperature_celsius = Some(chamber_end_temperature)
    });
    ctx.save();

    let parameters = fit_parameters(
        gain_bound,
        offset_bound,
        chamber_start_temperature,
        chamber_end_temperature,
    );
    ctx.section("Fitting the setpoints", |ctx| {
        fit_setpoints(ctx, parameters)
    })
    .await
}

/// Fits every setpoint's resistance against the square of its current,
/// extrapolating to zero current, and shows the result and its uncertainty
/// budget.
async fn fit_setpoints(
    ctx: Context,
    parameters: ColdResistanceFitParameters,
) -> Result<(), ProcedureError> {
    // Recorded before the fit runs, so the fit can be re-run from the results
    // file alone even if it fails.
    let mut points = None;
    record_cold_resistance(&ctx, |cold_resistance| {
        cold_resistance.fit_parameters = Some(parameters);
        points = cold_resistance
            .points
            .iter()
            .map(|point| {
                point
                    .analysis
                    .as_ref()
                    .map(|analysis| ColdResistanceFitPoint {
                        current_amps: analysis.current_amps,
                        voltage_volts: analysis.voltage_volts,
                    })
            })
            .collect::<Option<Vec<_>>>();
    });
    ctx.save();

    // Can't happen: a setpoint whose analysis fails ends the run.
    let Some(points) = points else {
        error!("a setpoint has no analysis");
        return Err(ProcedureError::Check(String::from(
            "a setpoint has no analysis, so the setpoints can't be fitted",
        )));
    };

    let analysis =
        python::cold_resistance_fit(&ColdResistanceFitInput { parameters, points }).await?;

    let r0 = analysis.resistance_ohms;
    let r20 = analysis.reference_resistance_ohms;
    ctx.measurement(
        format!(
            "Cold resistance at {:.0} °C",
            parameters.reference_temperature_celsius
        ),
        resistance(r20.value, r20.uncertainty),
    );
    ctx.measurement(
        format!(
            "Cold resistance at {:.1} °C",
            parameters.filament_temperature_celsius
        ),
        resistance(r0.value, r0.uncertainty),
    );

    // The budget, in ohms and as a percentage of R0.
    for (label, uncertainty) in [
        ("Statistical uncertainty", analysis.fit_uncertainty_ohms),
        (
            "Oscilloscope gain uncertainty",
            analysis.voltage_gain_uncertainty_ohms,
        ),
        (
            "Supply readback gain uncertainty",
            analysis.current_gain_uncertainty_ohms,
        ),
        (
            "Supply readback offset uncertainty",
            analysis.current_offset_uncertainty_ohms,
        ),
        (
            "Filament temperature uncertainty",
            analysis.temperature_uncertainty_ohms,
        ),
        (
            "Temperature coefficient uncertainty",
            analysis.temperature_coefficient_uncertainty_ohms,
        ),
    ] {
        ctx.measurement(
            label,
            format!(
                "{:.3} mΩ ({:.2} %)",
                uncertainty * 1000.0,
                uncertainty / r0.value * 100.0
            ),
        );
    }

    // The slope measures how strongly the filament heats itself.
    let slope = analysis.slope_ohms_per_amp_squared;
    ctx.measurement(
        "Slope",
        format!("{:.5} ± {:.5} Ω/A²", slope.value, slope.uncertainty),
    );
    ctx.measurement(
        "Reduced chi-squared",
        format!(
            "{:.2} (p = {:.3})",
            analysis.reduced_chi_squared, analysis.chi_squared_p_value
        ),
    );

    for warning in &analysis.warnings {
        warn!("{}", warning);
        ctx.text(format!("Warning: {}", warning));
    }

    // The data sheet bounds the scope's error without saying it's a pure gain
    // (COLD_RESISTANCE.md, 3.4.3), so the result rests on that assumption
    // until the scope's response is measured.
    ctx.text(
        "Note: the oscilloscope's gain uncertainty assumes its error is a pure gain, which its data sheet doesn't guarantee",
    );

    record_cold_resistance(&ctx, |cold_resistance| {
        cold_resistance.analysis = Some(analysis)
    });
    ctx.save();
    Ok(())
}

/// Chooses the smallest vertical scale on which the filament voltage doesn't
/// clip at the largest setpoint, and checks that the voltage reverses with the
/// current.
///
/// Returns the scale, which every measurement then uses, and the polarity that
/// gives a positive voltage. Leaves the output enabled at 0 A with the relays
/// reversed.
///
/// The scope's gain error is a single factor on every voltage, and so on the
/// resistance, only if every measurement is on the same scale, because each
/// scale has its own gain error. So it's chosen once here and never changed.
async fn choose_vertical_scale(
    ctx: Context,
    hardware: Hardware,
) -> Result<(f64, Polarity), ProcedureError> {
    let filament = &hardware.filament;
    let largest = largest_setpoint();
    let mut scale_index = 0;

    filament
        .set_vertical_scale(VERTICAL_SCALES_VOLTS_PER_DIVISION[0])
        .await?;

    // The relays are switched with the output off.
    filament.set_output_enabled(false).await?;
    filament.set_polarity(Polarity::Forward).await?;
    filament.set_output_enabled(true).await?;
    ramp_to(&ctx, &hardware, largest).await?;

    // Nothing is read during the ramp: the largest setpoint gives the largest
    // voltage, so a scale that fits it fits everything, and clipping on the
    // way up does no harm. Each reading waits for the supply to apply the
    // current first, so it's really at the largest setpoint.
    let (forward, reverse) = ctx
        .waiting("Reading the voltage in both polarities", || async {
            tokio::time::sleep(SETPOINT_APPLY_TIME).await;
            let forward = read_unclipped(&hardware, &mut scale_index).await?;

            change_polarity(&hardware, Polarity::Reverse).await?;
            tokio::time::sleep(SETPOINT_APPLY_TIME).await;
            let reverse = read_unclipped(&hardware, &mut scale_index).await?;

            Ok((forward, reverse))
        })
        .await?;

    let scale = VERTICAL_SCALES_VOLTS_PER_DIVISION[scale_index];

    // Forward = +V + offset and reverse = -V + offset, where the offset is
    // everything that doesn't reverse, so half their sum is the offset. On a
    // correctly wired rig that's the scope's offset error plus microvolts of
    // thermal EMF. Anything larger, or voltages with the same sign, means
    // something other than the filament contributes (a ground clip on the wrong
    // side of the relays, a ground loop, pickup) in a way reversal can't fix.
    // Both use readings already taken: if the scale went up for the reverse
    // reading, the forward one was on a smaller scale, which doesn't matter
    // for a sign check.
    let offset = (forward + reverse) / 2.0;
    if forward * reverse >= 0.0 || offset.abs() > offset_limit_volts(scale) {
        error!(
            "the filament voltage doesn't reverse with the current: {} V forward, {} V reversed",
            forward, reverse
        );
        return Err(ProcedureError::Check(String::from(
            "the filament voltage doesn't reverse with the current; check the wiring",
        )));
    }

    let positive_polarity = if forward > 0.0 {
        Polarity::Forward
    } else {
        Polarity::Reverse
    };

    ramp_to(&ctx, &hardware, 0.0).await?;
    Ok((scale, positive_polarity))
}

/// Measures one setpoint in both polarities, starting in `first`, and analyses
/// it. Returns the polarity it leaves the relays in, the other one.
///
/// The measurements are filed by the sign of their voltage, using
/// `positive_polarity`. That's the only place the relay-to-sign mapping is
/// used.
async fn measure_setpoint(
    ctx: Context,
    hardware: Hardware,
    setpoint: f64,
    first: Polarity,
    positive_polarity: Polarity,
    scale: f64,
) -> Result<Polarity, ProcedureError> {
    let second = if first == Polarity::Forward {
        Polarity::Reverse
    } else {
        Polarity::Forward
    };

    ramp_to(&ctx, &hardware, setpoint).await?;
    let first_measurement = measure_polarity(&ctx, &hardware, scale).await?;

    change_polarity(&hardware, second).await?;
    ctx.text(format!("Changed the polarity to {}", second));
    let second_measurement = measure_polarity(&ctx, &hardware, scale).await?;

    let (positive, negative) = if first == positive_polarity {
        (first_measurement, second_measurement)
    } else {
        (second_measurement, first_measurement)
    };

    let point = ColdResistancePoint {
        analysis: None,
        first_polarity: first,
        negative_current_amps: negative.current,
        negative_settle_seconds: negative.settle_seconds,
        negative_voltage_volts: negative.voltage,
        positive_current_amps: positive.current,
        positive_settle_seconds: positive.settle_seconds,
        positive_voltage_volts: positive.voltage,
        setpoint_amps: setpoint,
        warnings: positive
            .warning
            .into_iter()
            .chain(negative.warning)
            .collect(),
    };
    let input = ColdResistancePointInput {
        current_resolution_amps: CURRENT_READBACK_RESOLUTION_AMPS,
        negative_current_amps: point.negative_current_amps.clone(),
        negative_voltage_volts: point.negative_voltage_volts.clone(),
        offset_warning_volts: offset_limit_volts(scale),
        positive_current_amps: point.positive_current_amps.clone(),
        positive_voltage_volts: point.positive_voltage_volts.clone(),
    };

    // Saved before it's analysed, so the measurements survive an analysis
    // that fails.
    record_cold_resistance(&ctx, |cold_resistance| cold_resistance.points.push(point));
    ctx.save();

    let analysis = python::cold_resistance_point(&input).await?;
    ctx.measurement(
        "Resistance",
        format!(
            "{:.2} ± {:.2} mΩ",
            analysis.resistance_ohms.value * 1000.0,
            analysis.resistance_ohms.uncertainty * 1000.0
        ),
    );
    for warning in &analysis.warnings {
        warn!("{}", warning);
        ctx.text(format!("Warning: {}", warning));
    }

    record_cold_resistance(&ctx, |cold_resistance| {
        if let Some(point) = cold_resistance.points.last_mut() {
            point.analysis = Some(analysis);
        }
    });
    ctx.save();

    Ok(second)
}

/// Waits for the filament to settle at the present current and polarity, then
/// samples its voltage and current.
async fn measure_polarity(
    ctx: &Context,
    hardware: &Hardware,
    scale: f64,
) -> Result<PolarityMeasurement, ProcedureError> {
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

    let (voltages, currents) = take_samples(ctx, hardware, scale, SAMPLE_COUNT).await?;
    Ok(PolarityMeasurement {
        current: summarise(currents).await?,
        settle_seconds: settle_time.as_secs_f64(),
        voltage: summarise(voltages).await?,
        warning,
    })
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

/// The supply's current readback bounds, gain (relative) and offset (in
/// amperes), at `room_temperature_celsius`.
///
/// The data sheet specifies readback accuracy at 25 °C ± 5 °C and gives a
/// temperature coefficient separately, without saying how to combine them.
/// This follows the usual convention: the coefficient is added for each degree
/// outside the band.
fn current_readback_bounds(room_temperature_celsius: f64) -> (f64, f64) {
    let outside = (20.0 - room_temperature_celsius)
        .max(room_temperature_celsius - 30.0)
        .max(0.0);
    (
        CURRENT_GAIN_BOUND + CURRENT_GAIN_TEMPERATURE_COEFFICIENT_PER_CELSIUS * outside,
        CURRENT_OFFSET_BOUND_AMPS
            + CURRENT_OFFSET_TEMPERATURE_COEFFICIENT_AMPS_PER_CELSIUS * outside,
    )
}

/// The fit's scalar inputs, from the supply's current readback bounds (see
/// `current_readback_bounds`) and the chamber temperatures the operator
/// entered.
///
/// The filament temperature is the mean of the chamber's start and end
/// temperatures, and its bound covers the thermometer and placement plus half
/// the drift over the run. The linear sum is deliberately conservative.
fn fit_parameters(
    current_gain_bound: f64,
    current_offset_bound_amps: f64,
    chamber_start_temperature_celsius: f64,
    chamber_end_temperature_celsius: f64,
) -> ColdResistanceFitParameters {
    ColdResistanceFitParameters {
        current_gain_bound,
        current_offset_bound_amps,
        filament_temperature_bound_kelvin: THERMOMETER_BOUND_KELVIN
            + (chamber_end_temperature_celsius - chamber_start_temperature_celsius).abs() / 2.0,
        filament_temperature_celsius: (chamber_start_temperature_celsius
            + chamber_end_temperature_celsius)
            / 2.0,
        reference_temperature_celsius: REFERENCE_TEMPERATURE_CELSIUS,
        temperature_coefficient_bound_per_kelvin: TUNGSTEN_TEMPERATURE_COEFFICIENT_BOUND_PER_KELVIN,
        temperature_coefficient_per_kelvin: TUNGSTEN_TEMPERATURE_COEFFICIENT_PER_KELVIN,
        voltage_gain_bound: VOLTAGE_GAIN_BOUND,
    }
}

/// The largest of `COLD_RESISTANCE_SETPOINTS_AMPS`.
fn largest_setpoint() -> f64 {
    COLD_RESISTANCE_SETPOINTS_AMPS
        .iter()
        .copied()
        .fold(f64::MIN, f64::max)
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

/// The largest plausible voltage that doesn't reverse with the current, on
/// `scale`: twice the scope's DC offset spec, ±(0.1 div + 2 mV), since thermal
/// EMFs are only microvolts. Generous, because it's a plausibility check: if it
/// ever proves too tight on a correctly wired rig, loosen it rather than
/// removing it.
fn offset_limit_volts(scale: f64) -> f64 {
    2.0 * (0.1 * scale + 0.002)
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

/// Reads the filament voltage's average, stepping the vertical scale up through
/// `VERTICAL_SCALES_VOLTS_PER_DIVISION` while it clips. `scale_index` is the
/// scale in use, and is left at the one that didn't clip.
async fn read_unclipped(
    hardware: &Hardware,
    scale_index: &mut usize,
) -> Result<f64, ProcedureError> {
    let filament = &hardware.filament;
    loop {
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

/// A resistance and its standard uncertainty for display, with the expanded
/// uncertainty at a coverage factor of 2 (about 95%).
fn resistance(value: f64, uncertainty: f64) -> String {
    format!(
        "{:.3} ± {:.3} mΩ (U = {:.3} mΩ, k = 2)",
        value * 1000.0,
        uncertainty * 1000.0,
        2.0 * uncertainty * 1000.0
    )
}

/// Changes the cold resistance's results.
///
/// Panics if they haven't been created, which the setup at the start of
/// `measure_cold_resistance` does before anything calls this.
fn record_cold_resistance(ctx: &Context, f: impl FnOnce(&mut ColdResistance)) {
    ctx.record(|characterisation| {
        f(characterisation
            .cold_resistance
            .as_mut()
            .expect("the cold resistance results were created during setup"))
    });
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
    use crate::{
        hardware::{self, MockFilamentSystem, MockVacuumSystem},
        procedure,
        results::Characterisation,
        steps::{Section, StepKind},
    };
    use std::{
        fs,
        sync::{Arc, Mutex},
    };
    use tokio::sync::watch;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn current_readback_bounds_widen_outside_20_to_30_celsius() {
        let (gain, offset) = current_readback_bounds(25.0);
        assert_eq!(gain, CURRENT_GAIN_BOUND);
        assert_eq!(offset, CURRENT_OFFSET_BOUND_AMPS);

        // At the band's edges, nothing is added.
        assert_eq!(current_readback_bounds(20.0), (gain, offset));
        assert_eq!(current_readback_bounds(30.0), (gain, offset));

        // 5 °C below and 3 °C above.
        let (gain, offset) = current_readback_bounds(15.0);
        assert!((gain - 0.002).abs() < 1e-12);
        assert!((offset - 0.015).abs() < 1e-12);

        let (gain, offset) = current_readback_bounds(33.0);
        assert!((gain - 0.0018).abs() < 1e-12);
        assert!((offset - 0.011).abs() < 1e-12);
    }

    #[test]
    fn fit_parameters_combine_the_temperatures() {
        // The chamber warmed by 2 °C over the run.
        let parameters = fit_parameters(0.002, 0.011, 22.0, 24.0);
        assert_eq!(parameters.filament_temperature_celsius, 23.0);
        assert_eq!(
            parameters.filament_temperature_bound_kelvin,
            THERMOMETER_BOUND_KELVIN + 1.0
        );
        assert_eq!(parameters.current_gain_bound, 0.002);
        assert_eq!(parameters.current_offset_bound_amps, 0.011);
        assert_eq!(
            parameters.reference_temperature_celsius,
            REFERENCE_TEMPERATURE_CELSIUS
        );
    }

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

    /// Answers whichever prompt is waiting, as the operator would: confirms a
    /// gate, or enters a value for an input.
    fn answer_prompt(root: &Mutex<Section>) {
        let mut root = root.lock().unwrap();
        let Some(step) = root.pending_mut() else {
            return;
        };

        match &mut step.kind {
            StepKind::Confirm { responder, .. } => {
                if let Some(responder) = responder.take() {
                    let _ = responder.send(());
                }
            }
            StepKind::Input {
                prompt, responder, ..
            } => {
                let value = match prompt.as_str() {
                    "Filament ID" => "test",
                    "Room temperature near the power supply" => "21",
                    _ => "22",
                };
                if let Some(responder) = responder.take() {
                    let _ = responder.send(String::from(value));
                }
            }
            _ => {}
        }
    }

    // The whole procedure against the mocks, whose filament is 0.096 Ω with no
    // self-heating. The clock is paused, so tokio skips through the ramps and
    // settles instead of waiting minutes for them. It needs the Python virtual
    // environment, for the analysis scripts.
    #[tokio::test(start_paused = true)]
    async fn the_procedure_measures_the_mock_filament() {
        let hardware = Hardware {
            filament: Arc::new(MockFilamentSystem::default()),
            vacuum: Arc::new(MockVacuumSystem::default()),
        };
        let root = Arc::new(Mutex::new(Section::default()));
        let (snapshots_tx, snapshots_rx) = watch::channel(None);
        let ctx = Context::new(
            CancellationToken::new(),
            Characterisation::new(),
            Arc::clone(&root),
            snapshots_rx,
        );

        let operator = async {
            loop {
                answer_prompt(&root);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        };

        let result = tokio::select! {
            result = procedure::run(ctx.clone(), hardware.clone()) => result,
            e = hardware::poll(hardware.clone(), &snapshots_tx) => panic!("the poll failed: {}", e),
            _ = operator => unreachable!("the operator never stops"),
        };

        // Saving writes to `out/`, which a test shouldn't leave behind.
        let characterisation = ctx.characterisation.lock().unwrap();
        let _ = fs::remove_file(characterisation.path());
        result.unwrap();

        let cold_resistance = characterisation.cold_resistance.as_ref().unwrap();
        assert_eq!(cold_resistance.points.len(), 9);
        assert!(
            cold_resistance
                .points
                .iter()
                .all(|point| point.analysis.is_some())
        );

        let resistance = cold_resistance
            .analysis
            .as_ref()
            .unwrap()
            .resistance_ohms
            .value;
        assert!(
            (resistance - 0.096).abs() < 1e-9,
            "expected 0.096 Ω, got {} Ω",
            resistance
        );
    }
}
