//! Measuring the cold resistance: the setpoints in both polarities, with
//! references to track the mount's warming, and the fit.
//! See `filament-characterisation/docs/2_COLD_RESISTANCE.md` and
//! `filament-characterisation/docs/4_TEMPERATURE_COMPENSATION.md`.

use super::measurement::{
    CURRENT_READBACK_RESOLUTION_AMPS, OVERCURRENT_PROTECTION_MARGIN_AMPS, SAMPLE_COUNT,
    SETPOINT_APPLY_TIME, Settle, VERTICAL_SCALES_VOLTS_PER_DIVISION, change_polarity, ramp_to,
    read_unclipped, summarise, take_samples, wait_for_settle,
};
use crate::{
    hardware::Hardware,
    procedure::{Context, ProcedureError},
    python::{self, ColdResistanceFitInput, ColdResistanceFitPoint, ColdResistancePointInput},
    results::{ColdResistance, ColdResistanceFitParameters, ColdResistancePoint, Measurement},
};
use host::power_supply::Polarity;
use log::*;
use rand::seq::SliceRandom;
use tokio::time::Instant;

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

/// The current the references are measured at in amperes.
///
/// Mid-range: a good voltage without adding as much heat to every reference as
/// the largest setpoint would. It's also measured as an ordinary setpoint.
pub(super) const REFERENCE_CURRENT_AMPS: f64 = 0.2;

/// How many setpoints are measured between references, after the reference
/// that follows the first setpoint.
///
/// The mount warms with a time constant of roughly 2.5 minutes, so it drifts
/// fastest at the start: the extra early reference follows that, and this sets
/// the spacing after it. Shorten it if the interpolation uncertainty dominates
/// the points' uncertainties; lengthen it if it's negligible.
const REFERENCE_INTERVAL: usize = 2;

/// The temperature the cold resistance is corrected to in °C.
const REFERENCE_TEMPERATURE_CELSIUS: f64 = 20.0;

/// The bound on the filament temperature in kelvin: the thermometer's accuracy
/// plus how far the filament might sit from the flange's temperature.
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

/// The bound on the oscilloscope's gain error, relative.
///
/// The data sheet's ±1% of full scale is a bound on the absolute error at any
/// reading. Under the pure-gain model it bounds the gain at 2% read against a
/// single reading, the most that fits on screen, or 1% read against the
/// difference between a reading at the top of the screen and one at the
/// bottom. The second matches the measurement here, which takes half the
/// difference between two polarities, but the first is what the data sheet
/// literally guarantees (`2_COLD_RESISTANCE.md`, 3.4.3).
const VOLTAGE_GAIN_BOUND: f64 = 0.02;

/// The measurements in one polarity at one setpoint.
struct PolarityMeasurement {
    current: Measurement,
    settle_seconds: f64,

    /// The midpoint of the sampling window, in seconds since the
    /// cold-resistance measurement started.
    time_seconds: f64,

    voltage: Measurement,

    /// Set if the filament didn't settle in time.
    warning: Option<String>,
}

/// A type of measurement in the cold-resistance run. `measurement_plan` lists
/// them in the order they're measured.
#[derive(Clone, Copy, Debug, PartialEq)]
enum MeasurementType {
    /// A measurement at `REFERENCE_CURRENT_AMPS`, to track the mount's
    /// warming.
    Reference,

    /// A measurement at this setpoint in amperes, for the fit.
    Setpoint(f64),
}

/// Where `measure_setpoint` records what it measures.
#[derive(Clone, Copy)]
enum MeasurementSlot {
    Points,
    References,
}

impl MeasurementSlot {
    fn of(self, cold_resistance: &mut ColdResistance) -> &mut Vec<ColdResistancePoint> {
        match self {
            MeasurementSlot::Points => &mut cold_resistance.points,
            MeasurementSlot::References => &mut cold_resistance.references,
        }
    }
}

/// Measures the filament's resistance at each setpoint, in both directions.
///
/// Reversing the direction cancels everything that doesn't reverse with the
/// current, such as the Seebeck voltages at the filament's junctions and the
/// scope's offset.
pub(super) async fn measure_cold_resistance(
    ctx: Context,
    hardware: Hardware,
) -> Result<(), ProcedureError> {
    let filament = &hardware.filament;

    // Every measurement's time is relative to this.
    let start = Instant::now();

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
        .input(filament_temperature_prompt(&ctx), Some("°C"))
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
            references: Vec::new(),
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
    // scale search leaves the relays reversed. The references alternate along
    // with the setpoints.
    //
    // The filament's own power warms its mount, raising every reading over
    // the run. The references, at a fixed current, track that so the fit can
    // remove it.
    //
    // Setpoints and references are numbered together, in the order they're
    // measured, so the count shows how far through the run is.
    let plan = measurement_plan(&setpoints);
    let total = plan.len();
    let mut first_reference = None;
    let mut polarity = Polarity::Reverse;
    for (index, measure) in plan.into_iter().enumerate() {
        let number = index + 1;
        match measure {
            MeasurementType::Setpoint(setpoint) => {
                let title = format!(
                    "Measuring at {:.0} mA ({}/{})",
                    setpoint * 1000.0,
                    number,
                    total
                );
                (polarity, _) = ctx
                    .section(title, |ctx| {
                        measure_setpoint(
                            ctx,
                            hardware.clone(),
                            setpoint,
                            polarity,
                            positive_polarity,
                            scale,
                            start,
                            MeasurementSlot::Points,
                        )
                    })
                    .await?;
            }
            MeasurementType::Reference => {
                let title = format!(
                    "Measuring the reference at {:.0} mA ({}/{})",
                    REFERENCE_CURRENT_AMPS * 1000.0,
                    number,
                    total
                );
                let resistance;
                (polarity, resistance) = ctx
                    .section(title, |ctx| {
                        let hardware = hardware.clone();
                        async move {
                            let (next, resistance) = measure_setpoint(
                                ctx.clone(),
                                hardware,
                                REFERENCE_CURRENT_AMPS,
                                polarity,
                                positive_polarity,
                                scale,
                                start,
                                MeasurementSlot::References,
                            )
                            .await?;
                            if let Some(first) = first_reference {
                                ctx.measurement(
                                    "Drift since the first reference",
                                    format!("{:+.2} mΩ", (resistance - first) * 1000.0),
                                );
                            }
                            Ok((next, resistance))
                        }
                    })
                    .await?;
                first_reference.get_or_insert(resistance);
            }
        }
    }

    ramp_to(&ctx, &hardware, 0.0).await?;
    filament.set_output_enabled(false).await?;
    ctx.text("Disabled the output");

    let chamber_end_temperature: f64 = ctx
        .input(filament_temperature_prompt(&ctx), Some("°C"))
        .await?;
    record_cold_resistance(&ctx, |cold_resistance| {
        cold_resistance.chamber_end_temperature_celsius = Some(chamber_end_temperature)
    });
    ctx.save();

    let parameters = fit_parameters(gain_bound, offset_bound, chamber_start_temperature);
    ctx.section("Fitting the setpoints", |ctx| {
        fit_setpoints(ctx, parameters)
    })
    .await
}

/// Corrects every setpoint's resistance for the mount's drift, using the
/// references, then fits it against the square of its current, extrapolating
/// to zero current, and shows the result and its uncertainty budget.
async fn fit_setpoints(
    ctx: Context,
    parameters: ColdResistanceFitParameters,
) -> Result<(), ProcedureError> {
    // The fit's input for a setpoint or reference, or `None` if it has no
    // analysis.
    fn fit_point(point: &ColdResistancePoint) -> Option<ColdResistanceFitPoint> {
        let analysis = point.analysis.as_ref()?;
        Some(ColdResistanceFitPoint {
            current_amps: analysis.current_amps,
            time_seconds: (point.positive_time_seconds + point.negative_time_seconds) / 2.0,
            voltage_volts: analysis.voltage_volts,
        })
    }

    // Recorded before the fit runs, so the fit can be re-run from the results
    // file alone even if it fails.
    let mut input = None;
    record_cold_resistance(&ctx, |cold_resistance| {
        cold_resistance.fit_parameters = Some(parameters);
        let points = cold_resistance.points.iter().map(fit_point).collect();
        let references = cold_resistance.references.iter().map(fit_point).collect();
        if let (Some(points), Some(references)) = (points, references) {
            input = Some(ColdResistanceFitInput {
                parameters,
                points,
                references,
            });
        }
    });
    ctx.save();

    // Can't happen: a measurement whose analysis fails ends the run.
    let Some(input) = input else {
        error!("a setpoint or reference has no analysis");
        return Err(ProcedureError::Check(String::from(
            "a setpoint or reference has no analysis, so the setpoints can't be fitted",
        )));
    };

    let analysis = python::cold_resistance_fit(&input).await?;

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

    let drift = analysis.reference_drift_ohms;
    ctx.measurement(
        "Drift at the reference current over the run",
        format!(
            "{:+.3} ± {:.3} mΩ",
            drift.value * 1000.0,
            drift.uncertainty * 1000.0
        ),
    );

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
    // (`2_COLD_RESISTANCE.md`, 3.4.3), so the result rests on that assumption
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
            let forward = read_unclipped(&ctx, &hardware, &mut scale_index).await?;

            change_polarity(&hardware, Polarity::Reverse).await?;
            tokio::time::sleep(SETPOINT_APPLY_TIME).await;
            let reverse = read_unclipped(&ctx, &hardware, &mut scale_index).await?;

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

/// Measures one setpoint in both polarities, starting in `first`, analyses it,
/// and records it in `slot`. Returns the polarity it leaves the relays in,
/// which is the opposite of `first`, and its resistance in ohms. Times are
/// recorded relative to `start`.
///
/// The measurements are filed by the sign of their voltage, using
/// `positive_polarity`. That's the only place the relay-to-sign mapping is
/// used.
#[expect(clippy::too_many_arguments)]
async fn measure_setpoint(
    ctx: Context,
    hardware: Hardware,
    setpoint: f64,
    first: Polarity,
    positive_polarity: Polarity,
    scale: f64,
    start: Instant,
    slot: MeasurementSlot,
) -> Result<(Polarity, f64), ProcedureError> {
    let second = if first == Polarity::Forward {
        Polarity::Reverse
    } else {
        Polarity::Forward
    };

    ramp_to(&ctx, &hardware, setpoint).await?;
    let first_measurement = measure_polarity(&ctx, &hardware, scale, start).await?;

    change_polarity(&hardware, second).await?;
    ctx.text(format!("Changed the polarity to {}", second));
    let second_measurement = measure_polarity(&ctx, &hardware, scale, start).await?;

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
        negative_time_seconds: negative.time_seconds,
        negative_voltage_volts: negative.voltage,
        positive_current_amps: positive.current,
        positive_settle_seconds: positive.settle_seconds,
        positive_time_seconds: positive.time_seconds,
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
    record_cold_resistance(&ctx, |cold_resistance| slot.of(cold_resistance).push(point));
    ctx.save();

    let analysis = python::cold_resistance_point(&input).await?;
    let resistance = analysis.resistance_ohms.value;
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
        if let Some(point) = slot.of(cold_resistance).last_mut() {
            point.analysis = Some(analysis);
        }
    });
    ctx.save();

    Ok((second, resistance))
}

/// Waits for the filament to settle at the present current and polarity, then
/// samples its voltage and current. The time is recorded relative to `start`.
async fn measure_polarity(
    ctx: &Context,
    hardware: &Hardware,
    scale: f64,
    start: Instant,
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

    let (voltages, currents, time_seconds) =
        take_samples(ctx, hardware, scale, SAMPLE_COUNT, start).await?;
    Ok(PolarityMeasurement {
        current: summarise(currents).await?,
        settle_seconds: settle_time.as_secs_f64(),
        time_seconds,
        voltage: summarise(voltages).await?,
        warning,
    })
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

/// The prompt for the temperature the unheated filament sits at: the chamber's
/// at the filament's flange, or without vacuum (`--no-vacuum`), the air's.
fn filament_temperature_prompt(ctx: &Context) -> &'static str {
    if ctx.in_vacuum() {
        "Chamber temperature at the filament's flange"
    } else {
        "Air temperature around the filament"
    }
}

/// The fit's scalar inputs, from the supply's current readback bounds (see
/// `current_readback_bounds`) and the chamber temperature the operator entered
/// before the setpoints.
///
/// The filament temperature is the chamber's at the start: the references
/// correct for any warming after that, whether from the filament or the
/// chamber, so the end temperature isn't needed.
fn fit_parameters(
    current_gain_bound: f64,
    current_offset_bound_amps: f64,
    chamber_start_temperature_celsius: f64,
) -> ColdResistanceFitParameters {
    ColdResistanceFitParameters {
        current_gain_bound,
        current_offset_bound_amps,
        filament_temperature_bound_kelvin: THERMOMETER_BOUND_KELVIN,
        filament_temperature_celsius: chamber_start_temperature_celsius,
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

/// The order to measure `setpoints` and the references in: a reference first,
/// after the first setpoint, after every `REFERENCE_INTERVAL` setpoints after
/// that, and last, so every setpoint has a reference before and after it.
fn measurement_plan(setpoints: &[f64]) -> Vec<MeasurementType> {
    let mut plan = vec![MeasurementType::Reference];
    for (index, &setpoint) in setpoints.iter().enumerate() {
        plan.push(MeasurementType::Setpoint(setpoint));
        if index % REFERENCE_INTERVAL == 0 || index == setpoints.len() - 1 {
            plan.push(MeasurementType::Reference);
        }
    }
    plan
}

/// The largest plausible voltage that doesn't reverse with the current, on
/// `scale`: twice the scope's DC offset spec, ±(0.1 div + 2 mV), since thermal
/// EMFs are only microvolts. Generous, because it's a plausibility check: if it
/// ever proves too tight on a correctly wired rig, loosen it rather than
/// removing it.
fn offset_limit_volts(scale: f64) -> f64 {
    2.0 * (0.1 * scale + 0.002)
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn fit_parameters_use_the_start_temperature() {
        let parameters = fit_parameters(0.002, 0.011, 22.0);
        assert_eq!(parameters.filament_temperature_celsius, 22.0);
        assert_eq!(
            parameters.filament_temperature_bound_kelvin,
            THERMOMETER_BOUND_KELVIN
        );
        assert_eq!(parameters.current_gain_bound, 0.002);
        assert_eq!(parameters.current_offset_bound_amps, 0.011);
        assert_eq!(
            parameters.reference_temperature_celsius,
            REFERENCE_TEMPERATURE_CELSIUS
        );
    }

    #[test]
    fn measurement_plan_interleaves_references_with_nine_setpoints() {
        use MeasurementType::{Reference as R, Setpoint as S};
        let setpoints = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
        assert_eq!(
            measurement_plan(&setpoints),
            [
                R,
                S(1.0),
                R,
                S(2.0),
                S(3.0),
                R,
                S(4.0),
                S(5.0),
                R,
                S(6.0),
                S(7.0),
                R,
                S(8.0),
                S(9.0),
                R
            ]
        );
    }

    #[test]
    fn measurement_plan_brackets_every_setpoint_without_doubling_a_reference() {
        for n in 1..=12 {
            let setpoints: Vec<f64> = (1..=n).map(f64::from).collect();
            let plan = measurement_plan(&setpoints);

            assert_eq!(
                plan.first(),
                Some(&MeasurementType::Reference),
                "{} setpoints",
                n
            );
            assert_eq!(
                plan.last(),
                Some(&MeasurementType::Reference),
                "{} setpoints",
                n
            );
            assert!(
                plan.windows(2)
                    .all(|pair| pair != [MeasurementType::Reference, MeasurementType::Reference]),
                "{} setpoints: {:?}",
                n,
                plan
            );

            let measured: Vec<f64> = plan
                .iter()
                .filter_map(|measure| match measure {
                    MeasurementType::Setpoint(setpoint) => Some(*setpoint),
                    MeasurementType::Reference => None,
                })
                .collect();
            assert_eq!(measured, setpoints);
        }
    }
}
