//! Drives the filament with a short pulse of high current and logs the mount
//! cooling afterwards. See `filament-characterisation/docs/6_PULSE_HEATING.md`.
//!
//! A plain command-line program: every step prints a line to stdout. It doesn't
//! control the TMP, and only reads the chamber pressure to check that it's low
//! enough to power the filament.

use characteriser::{
    constants::{FILAMENT_OPERATING_PRESSURE_MBAR, POWER_SUPPLY_MAXIMUM_CURRENT_AMPS},
    hardware::{self, FilamentSystem, Hardware},
    python,
    results::save_json,
    sampling::{
        CLIPPING_HEADROOM, OVERCURRENT_PROTECTION_MARGIN_AMPS, RELAY_SETTLE_TIME,
        SETPOINT_APPLY_TIME, VERTICAL_SCALES_VOLTS_PER_DIVISION, unclipped_average,
    },
};
use clap::Parser;
use env_logger::{Builder, Target};
use futures::FutureExt;
use host::power_supply::{Polarity, RegulationMode};
use log::*;
use schemars::JsonSchema;
use serde::Serialize;
use std::{
    any::Any,
    fs,
    panic::AssertUnwindSafe,
    path::PathBuf,
    process::ExitCode,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::time::Instant;

type AnyError = Box<dyn std::error::Error>;

/// The longest pulse that may be asked for.
///
/// Longer than filming needs. A pulse this long at incandescent currents would
/// put tens of joules into the mount.
const MAXIMUM_PULSE_DURATION: Duration = Duration::from_secs(10);

/// The supply's voltage limit throughout, in volts.
///
/// Bounds the pulse's power, and sets the scope's scale for the pulse. Above
/// what 3 A needs unless the hot resistance exceeds 1.7 Ω.
const PULSE_VOLTAGE_LIMIT_VOLTS: f64 = 5.0;

/// The current the mount's temperature is measured with, in amperes.
///
/// About 1 mW, which would settle at about 0.45 K of warming: small enough to
/// ignore. 300 mA would warm the mount by several kelvin over the run.
const PROBE_CURRENT_AMPS: f64 = 0.1;

/// How long the probe resistance is recorded before the pulse.
///
/// Long enough to average the probe's noise down to about 0.1 K, and to check
/// that the mount isn't still cooling.
const BASELINE_DURATION: Duration = Duration::from_secs(60);

/// How long the probe resistance is recorded after the pulse.
///
/// About five of the mount's 2-minute time constants.
const COOL_DOWN_DURATION: Duration = Duration::from_secs(600);

/// The largest baseline slope, in ohms per second, that counts as the mount
/// having cooled.
///
/// A mount X above its cold resistance, cooling with τ ≈ 2 minutes, drifts at
/// X/τ, so this is X ≈ 0.36 mΩ, about 1 K warm. A tighter limit would trip on
/// noise: the fitted slope over 60 s is uncertain by about this much.
const BASELINE_MAXIMUM_SLOPE_OHMS_PER_SECOND: f64 = 3e-6;

/// How long making the filament safe may take before it's given up on.
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the probe stays in each polarity before switching.
///
/// The offset that reversal cancels follows the clamp's temperature, which
/// changes over minutes, so over 10 s it should move by a few µV at most. About
/// 6 reversals in the baseline and 60 in the cool-down, and about 20 samples
/// per block once `SETPOINT_APPLY_TIME` has passed.
const REVERSAL_BLOCK_DURATION: Duration = Duration::from_secs(10);

/// A current readback below this, in amperes, with the supply out of constant
/// current, is taken to mean the circuit is open.
///
/// A tenth of the probe current: far below anything a connected filament
/// draws.
const OPEN_CIRCUIT_CURRENT_AMPS: f64 = 0.01;

/// The filament's resistance with no current and the mount at room
/// temperature, in ohms, from the hot sweeps. Only used to print the clamp's
/// rise as the cool-down goes.
const COLD_RESISTANCE_OHMS: f64 = 0.0977;

/// W-Re's temperature coefficient of resistance near room temperature, per
/// kelvin. Only used to print the clamp's rise as the cool-down goes.
const TEMPERATURE_COEFFICIENT_PER_KELVIN: f64 = 0.0031;

#[derive(Parser)]
struct Arguments {
    /// The path to the controller device, e.g. /dev/tty.usbmodem11201.
    device_path: String,

    /// The gauge number to use when communicating with the ADC, e.g. 1.
    #[arg(value_parser = clap::value_parser!(u8).range(1..=2))]
    adc_gauge_number: u8,

    /// The address of the TMP in the RS-485 Pfeiffer Vacuum Protocol, e.g. 001.
    ///
    /// Needed only to connect to the vacuum system; the TMP is never commanded.
    tmp_address: String,

    /// An identifier for the filament, recorded in the results.
    #[arg(long)]
    filament_id: String,

    /// The pulse current in amperes.
    #[arg(long)]
    current: f64,

    /// How long the pulse is held, in seconds.
    #[arg(long)]
    duration: f64,

    /// Ends the pulse early once it has delivered this much energy, in joules.
    #[arg(long)]
    maximum_energy: Option<f64>,
}

/// Rejects arguments outside their limits, before anything is powered.
///
/// Written so that NaN fails every check.
fn check_arguments(args: &Arguments) -> Result<(), String> {
    if !(args.current > PROBE_CURRENT_AMPS && args.current <= POWER_SUPPLY_MAXIMUM_CURRENT_AMPS) {
        return Err(format!(
            "the current must be above the probe current, {} A, and at most {} A: {}",
            PROBE_CURRENT_AMPS, POWER_SUPPLY_MAXIMUM_CURRENT_AMPS, args.current
        ));
    }

    if !(args.duration > 0.0 && args.duration <= MAXIMUM_PULSE_DURATION.as_secs_f64()) {
        return Err(format!(
            "the duration must be more than 0 s and at most {} s: {}",
            MAXIMUM_PULSE_DURATION.as_secs_f64(),
            args.duration
        ));
    }

    if let Some(energy) = args.maximum_energy
        && (energy.is_nan() || energy <= 0.0)
    {
        return Err(format!(
            "the maximum energy must be more than 0 J: {}",
            energy
        ));
    }

    Ok(())
}

/// Everything recorded in a run, saved as `out/pulse-<started_at>.json`.
#[derive(Debug, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
struct PulseRun {
    /// The probe samples before the pulse.
    baseline: Vec<ProbeSample>,

    /// The probe samples after the pulse.
    cool_down: Vec<ProbeSample>,

    /// Why the run stopped early, if it did.
    error: Option<String>,

    /// An identifier for the filament, from the arguments.
    filament_id: String,

    /// The arguments and the constants in force.
    parameters: Parameters,

    /// The chamber pressure when it was checked.
    pressure_mbar: Pressures,

    /// The pulse, once its current has been set.
    pulse: Option<Pulse>,

    /// The scope's vertical scales, once chosen.
    scales: Scales,

    /// When the run started, in seconds since the Unix epoch.
    started_at: u64,

    /// Anything worth knowing that didn't stop the run.
    warnings: Vec<String>,
}

impl PulseRun {
    fn new(args: &Arguments) -> Self {
        Self {
            baseline: Vec::new(),
            cool_down: Vec::new(),
            error: None,
            filament_id: args.filament_id.clone(),
            parameters: Parameters {
                baseline_duration_seconds: BASELINE_DURATION.as_secs_f64(),
                baseline_maximum_slope_ohms_per_second: BASELINE_MAXIMUM_SLOPE_OHMS_PER_SECOND,
                cool_down_duration_seconds: COOL_DOWN_DURATION.as_secs_f64(),
                current_amps: args.current,
                duration_seconds: args.duration,
                maximum_energy_joules: args.maximum_energy,
                probe_current_amps: PROBE_CURRENT_AMPS,
                voltage_limit_volts: PULSE_VOLTAGE_LIMIT_VOLTS,
            },
            pressure_mbar: Pressures::default(),
            pulse: None,
            scales: Scales::default(),

            // Only fails if the system clock is set before 1970.
            started_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("the system clock is before the Unix epoch")
                .as_secs(),
            warnings: Vec::new(),
        }
    }

    /// Where the results are written, named for when the run started.
    fn path(&self) -> PathBuf {
        PathBuf::from(format!("out/pulse-{}.json", self.started_at))
    }
}

/// The arguments as given, and the constants in force.
#[derive(Debug, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
struct Parameters {
    baseline_duration_seconds: f64,
    baseline_maximum_slope_ohms_per_second: f64,
    cool_down_duration_seconds: f64,

    /// The pulse current asked for.
    current_amps: f64,

    /// The pulse duration asked for.
    duration_seconds: f64,

    maximum_energy_joules: Option<f64>,
    probe_current_amps: f64,
    voltage_limit_volts: f64,
}

/// The chamber pressure at each check, once taken.
#[derive(Debug, Default, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
struct Pressures {
    before_firing: Option<f64>,
    startup: Option<f64>,
}

/// The scope's vertical scales in volts per division, once chosen.
#[derive(Debug, Default, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
struct Scales {
    probe: Option<f64>,
    pulse: Option<f64>,
}

/// One sample at the probe current.
#[derive(Debug, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
struct ProbeSample {
    current_amps: f64,

    polarity: Polarity,

    /// When the sample finished, in seconds since the run started.
    time_seconds: f64,

    /// Negative in the reverse polarity.
    voltage_volts: f64,
}

/// A probe resistance from a neighbouring pair of blocks, one in each polarity.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ProbeResistance {
    resistance_ohms: f64,

    /// The midpoint of the two blocks' mean times, in seconds since the run
    /// started.
    time_seconds: f64,
}

/// The pulse: its samples, and when and how it started and ended.
#[derive(Debug, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
struct Pulse {
    /// When the current was set back down to the probe current, in seconds
    /// since the run started. `None` if the run stopped before then.
    down_seconds: Option<f64>,

    /// How the pulse ended. `None` if the run stopped during it.
    end: Option<PulseEnd>,

    /// The samples, in the order they were taken.
    samples: Vec<PulseSample>,

    /// When the current was set to the pulse current, in seconds since the
    /// run started.
    up_seconds: f64,
}

/// One sample during the pulse.
#[derive(Debug, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
struct PulseSample {
    current_amps: f64,

    /// When the sample finished, in seconds since the run started.
    time_seconds: f64,

    voltage_volts: f64,
}

/// Why the pulse ended.
#[derive(Clone, Copy, Debug, JsonSchema, PartialEq, Serialize)]
enum PulseEnd {
    /// The requested duration passed.
    Duration,

    /// `--maximum-energy` was reached.
    EnergyLimit,

    /// The supply left constant current, so the voltage limit bound: the hot
    /// resistance is more than the limit divided by the current.
    ConstantVoltage,
}

/// Reports `run`'s result and turns it into an exit status, printing errors
/// with `Display` rather than `Debug`.
#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {}", e);
            ExitCode::FAILURE
        }
    }
}

/// Runs the binary.
///
/// Every exit after the hardware is built goes through the same cleanup: the
/// sequence's result, whether it finished, failed, panicked or was interrupted
/// with Ctrl-C, is handled in one place, which always makes the filament safe
/// and saves the results.
async fn run() -> Result<(), AnyError> {
    init_logging()?;

    let args = Arguments::parse();
    check_arguments(&args)?;

    // Checked now so a broken environment is reported before anything is
    // powered, rather than when the analysis is added.
    println!("Checking the Python environment");
    python::check_environment().await?;

    println!("Connecting to the hardware");
    let hardware =
        hardware::build_hardware(&args.device_path, args.adc_gauge_number, &args.tmp_address)
            .await?;

    let mut pulse_run = PulseRun::new(&args);
    let start = Instant::now();
    let path = pulse_run.path();

    // The sequence is dropped if Ctrl-C wins, wherever it is. A command it was
    // in the middle of goes through the same instrument lock as the cleanup's,
    // so the cleanup's still come after.
    let result = tokio::select! {
        result = AssertUnwindSafe(sequence(&hardware, &args, &mut pulse_run, start)).catch_unwind() => {
            match result {
                Ok(result) => result.map_err(|e| e.to_string()),
                Err(panic) => Err(format!("panicked: {}", panic_message(panic.as_ref()))),
            }
        }
        _ = tokio::signal::ctrl_c() => Err(String::from("interrupted with Ctrl-C")),
    };

    println!("Making the filament safe");
    let cleanup =
        match tokio::time::timeout(CLEANUP_TIMEOUT, hardware.filament.enter_safe_state()).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => Err(format!(
                "cleanup didn't finish within {:?}",
                CLEANUP_TIMEOUT
            )),
        };

    if let Err(e) = &result {
        error!("the run stopped early: {}", e);
        pulse_run.error = Some(e.clone());
    }
    match save_json(&pulse_run, &path) {
        Ok(()) => println!("Results written to {}", path.display()),
        Err(e) => eprintln!("error: couldn't save the results: {}", e),
    }

    if let Err(e) = &cleanup {
        error!("couldn't make the filament safe: {}", e);
        eprintln!("Warning: the filament system may still be powered: {}", e);
    }

    Ok(result?)
}

/// Everything from the startup pressure check to the end of the cool-down.
///
/// Leaves the filament powered at the probe current on success and anywhere on
/// failure: the caller makes it safe.
async fn sequence(
    hardware: &Hardware,
    args: &Arguments,
    pulse_run: &mut PulseRun,
    start: Instant,
) -> Result<(), AnyError> {
    let filament = hardware.filament.as_ref();

    // 0. Startup.
    pulse_run.pressure_mbar.startup = Some(check_pressure(hardware).await?);

    // 1. Setup. The polarity can only be switched with the output off, and
    // the limits are set before it's turned on.
    println!("Setting up the supply");
    filament.set_output_enabled(false).await?;
    filament.set_polarity(Polarity::Forward).await?;
    filament
        .set_heating_voltage_limit(PULSE_VOLTAGE_LIMIT_VOLTS)
        .await?;
    filament
        .set_overcurrent_protection(args.current + OVERCURRENT_PROTECTION_MARGIN_AMPS)
        .await?;
    filament.set_heating_current_limit(0.0).await?;
    filament.set_output_enabled(true).await?;

    // The supply is constant-current, so there's no inrush to ramp around.
    println!("Setting the probe current, {} A", PROBE_CURRENT_AMPS);
    filament
        .set_heating_current_limit(PROBE_CURRENT_AMPS)
        .await?;
    tokio::time::sleep(SETPOINT_APPLY_TIME).await;

    let probe_scale = choose_probe_scale(filament).await?;
    pulse_run.scales.probe = Some(probe_scale);
    let mut polarity = Polarity::Forward;

    // 2. Baseline.
    println!(
        "Recording the baseline for {} s",
        BASELINE_DURATION.as_secs()
    );
    let baseline_start = Instant::now();
    record_probe(
        filament,
        probe_scale,
        BASELINE_DURATION,
        start,
        &mut polarity,
        &mut pulse_run.baseline,
        |resistances| {
            let elapsed = baseline_start.elapsed().as_secs_f64();
            if resistances.is_empty() {
                println!("  {:>3.0} s: first block recorded", elapsed);
            } else {
                println!(
                    "  {:>3.0} s: mean resistance so far {:.3} mΩ",
                    elapsed,
                    mean(resistances.iter().map(|r| r.resistance_ohms)) * 1e3
                );
            }
        },
    )
    .await?;

    let (baseline_resistance, slope) = baseline_fit(&probe_resistances(&pulse_run.baseline))
        .ok_or("the baseline has fewer than two reversed pairs of blocks")?;
    println!(
        "Baseline: {:.3} mΩ, slope {:.2} µΩ/s",
        baseline_resistance * 1e3,
        slope * 1e6
    );
    // A cooling mount's resistance falls. A warming one's rises, which nothing
    // in the run explains: the probe's own warming adds only about 1 µΩ/s.
    if slope.is_nan() {
        return Err("couldn't fit the baseline's slope".into());
    }
    if slope < -BASELINE_MAXIMUM_SLOPE_OHMS_PER_SECOND {
        return Err(format!(
            "the baseline is falling at {:.2} µΩ/s, faster than {:.0} µΩ/s, so the mount is \
             probably still cooling from an earlier run; wait and run again",
            -slope * 1e6,
            BASELINE_MAXIMUM_SLOPE_OHMS_PER_SECOND * 1e6
        )
        .into());
    }
    if slope > BASELINE_MAXIMUM_SLOPE_OHMS_PER_SECOND {
        return Err(format!(
            "the baseline is rising at {:.2} µΩ/s, faster than {:.0} µΩ/s, so the mount is \
             warming; check that nothing else is heating it and that the room's temperature \
             is steady, then run again",
            slope * 1e6,
            BASELINE_MAXIMUM_SLOPE_OHMS_PER_SECOND * 1e6
        )
        .into());
    }

    // The pulse is always in forward polarity.
    if polarity != Polarity::Forward {
        change_polarity(filament, Polarity::Forward).await?;
        polarity = Polarity::Forward;
    }

    // 3. Fire. The baseline took a minute, so the pressure is checked again.
    pulse_run.pressure_mbar.before_firing = Some(check_pressure(hardware).await?);

    // The scale changes before the pulse because changing it takes a second or
    // two.
    let pulse_scale = pulse_scale(PULSE_VOLTAGE_LIMIT_VOLTS);
    println!("Setting the scope to {} V/div for the pulse", pulse_scale);
    filament.set_vertical_scale(pulse_scale).await?;
    pulse_run.scales.pulse = Some(pulse_scale);

    // 4. The pulse.
    println!(
        "Firing: {} A for {} s{}",
        args.current,
        args.duration,
        match args.maximum_energy {
            Some(energy) => format!(", or until {} J", energy),
            None => String::new(),
        }
    );
    let energy = fire(
        filament,
        args.current,
        Duration::from_secs_f64(args.duration),
        args.maximum_energy,
        pulse_scale,
        start,
        &mut pulse_run.pulse,
    )
    .await?;
    let pulse = pulse_run.pulse.as_ref().expect("fire records the pulse");
    let down = pulse.down_seconds.expect("fire records the end");
    println!(
        "Pulse ended ({:?}) after {:.2} s, {:.2} J",
        pulse.end.expect("fire records the end"),
        down - pulse.up_seconds,
        energy
    );
    if pulse.end == Some(PulseEnd::ConstantVoltage) {
        println!(
            "  The supply left constant current: the hot resistance is more than {} V / {} A",
            PULSE_VOLTAGE_LIMIT_VOLTS, args.current
        );
    }

    // 5. Cool-down. The first second or two of samples are lost while the
    // scale changes back.
    filament.set_vertical_scale(probe_scale).await?;
    println!(
        "Recording the cool-down for {} s",
        COOL_DOWN_DURATION.as_secs()
    );
    let down_time = start + Duration::from_secs_f64(down);
    record_probe(
        filament,
        probe_scale,
        COOL_DOWN_DURATION,
        start,
        &mut polarity,
        &mut pulse_run.cool_down,
        |resistances| {
            // The latest pair, as a lower bound on the warming at its time.
            let Some(latest) = resistances.last() else {
                println!(
                    "  {:>3.0} s after the pulse: first block recorded",
                    down_time.elapsed().as_secs_f64()
                );
                return;
            };
            let extra = latest.resistance_ohms - baseline_resistance;
            println!(
                "  {:>3.0} s after the pulse: {:+.3} mΩ, {:+.1} K at the clamp (W-Re)",
                latest.time_seconds - down,
                extra * 1e3,
                extra / (COLD_RESISTANCE_OHMS * TEMPERATURE_COEFFICIENT_PER_KELVIN)
            );
        },
    )
    .await?;

    Ok(())
}

/// Reads the chamber pressure once, in mbar, failing unless it's below
/// `FILAMENT_OPERATING_PRESSURE_MBAR`.
///
/// The TMP's state isn't checked: a low pressure is what the filament needs,
/// however it was reached. The real vacuum system always reads in mbar.
async fn check_pressure(hardware: &Hardware) -> Result<f64, AnyError> {
    let pressure = hardware.vacuum.snapshot().await?.pressure.value;
    println!("Chamber pressure: {:.1e} mbar", pressure);
    pressure_ok(pressure)?;
    Ok(pressure)
}

/// Whether `pressure`, in mbar, is low enough to power the filament.
fn pressure_ok(pressure: f64) -> Result<(), String> {
    if pressure < FILAMENT_OPERATING_PRESSURE_MBAR {
        Ok(())
    } else {
        Err(format!(
            "the chamber pressure is {:.1e} mbar, but must be below {:.0e} mbar to power the \
             filament",
            pressure, FILAMENT_OPERATING_PRESSURE_MBAR
        ))
    }
}

/// Chooses the probe's vertical scale: the smallest on which a reading at the
/// probe current doesn't clip.
async fn choose_probe_scale(filament: &dyn FilamentSystem) -> Result<f64, AnyError> {
    for scale in VERTICAL_SCALES_VOLTS_PER_DIVISION {
        filament.set_vertical_scale(scale).await?;
        if unclipped_average(filament.get_filament_voltages().await?, scale).is_some() {
            println!("Probe scale: {} V/div", scale);
            return Ok(scale);
        }
    }
    Err("the probe voltage clips on every vertical scale".into())
}

/// The pulse's vertical scale: the smallest on which a reading at
/// `voltage_limit` doesn't clip. The supply can't put more than its voltage
/// limit across the filament, so no reading can clip.
fn pulse_scale(voltage_limit: f64) -> f64 {
    *VERTICAL_SCALES_VOLTS_PER_DIVISION
        .iter()
        .find(|&&scale| voltage_limit <= CLIPPING_HEADROOM * 4.0 * scale)
        .expect("the largest scale fits the voltage limit")
}

/// One reading of the filament's voltage and current.
struct Reading {
    current: f64,
    mode: RegulationMode,
    voltage: f64,
}

/// Takes one sample: the filament's voltage, its current, then whether the
/// supply's overcurrent protection has tripped and how it's regulating.
///
/// With a `deadline`, returns `Ok(None)` without starting the next instrument
/// call once it has passed, so a pulse overruns by at most the call in flight.
/// A call isn't abandoned partway, because the instruments' replies would be
/// left for the next query to read.
///
/// Fails if the protection has tripped, the circuit looks open, or the voltage
/// clipped. Leaving constant current otherwise isn't an error here: the caller
/// decides.
async fn sample(
    filament: &dyn FilamentSystem,
    scale: f64,
    deadline: Option<Instant>,
) -> Result<Option<Reading>, AnyError> {
    let passed = || deadline.is_some_and(|deadline| Instant::now() >= deadline);

    if passed() {
        return Ok(None);
    }
    let voltages = filament.get_filament_voltages().await?;

    if passed() {
        return Ok(None);
    }
    let current = filament.get_heating_current().await?;

    if passed() {
        return Ok(None);
    }
    if filament.get_overcurrent_tripped().await? {
        return Err("overcurrent protection tripped".into());
    }

    if passed() {
        return Ok(None);
    }
    let mode = filament.get_regulation_mode().await?;
    if mode != RegulationMode::ConstantCurrent && current < OPEN_CIRCUIT_CURRENT_AMPS {
        return Err(format!(
            "the supply left constant current ({:?}) with {:.4} A flowing, so the filament \
             circuit is probably open",
            mode, current
        )
        .into());
    }

    let Some(voltage) = unclipped_average(voltages, scale) else {
        return Err(format!(
            "the filament voltage clipped at {} V/div: {:?}",
            scale, voltages
        )
        .into());
    };

    Ok(Some(Reading {
        current,
        mode,
        voltage,
    }))
}

/// Records samples at the probe current into `samples` for `duration`, in
/// blocks of `REVERSAL_BLOCK_DURATION` that alternate polarity. The first
/// block is in `polarity`, which is left as the last block's. Calls `report`
/// with the probe resistances so far after each block.
///
/// After each reversal, waits `SETPOINT_APPLY_TIME` for the supply to settle
/// with the output back on before sampling.
async fn record_probe(
    filament: &dyn FilamentSystem,
    scale: f64,
    duration: Duration,
    start: Instant,
    polarity: &mut Polarity,
    samples: &mut Vec<ProbeSample>,
    report: impl Fn(&[ProbeResistance]),
) -> Result<(), AnyError> {
    let began = Instant::now();
    let mut first = true;

    while began.elapsed() < duration {
        let block_start = Instant::now();
        if !first {
            *polarity = if *polarity == Polarity::Forward {
                Polarity::Reverse
            } else {
                Polarity::Forward
            };
            change_polarity(filament, *polarity).await?;
            tokio::time::sleep(SETPOINT_APPLY_TIME).await;
        }
        first = false;

        while block_start.elapsed() < REVERSAL_BLOCK_DURATION && began.elapsed() < duration {
            let reading = sample(filament, scale, None)
                .await?
                .expect("there's no deadline");
            if reading.mode != RegulationMode::ConstantCurrent {
                return Err(format!(
                    "the supply left constant current ({:?}) at the probe current",
                    reading.mode
                )
                .into());
            }

            samples.push(ProbeSample {
                current_amps: reading.current,
                polarity: *polarity,
                time_seconds: start.elapsed().as_secs_f64(),
                voltage_volts: reading.voltage,
            });
        }

        report(&probe_resistances(samples));
    }

    Ok(())
}

/// Changes the direction of the current through the filament, leaving the
/// output on at the same setpoint.
///
/// The output is off for `RELAY_SETTLE_TIME` while the relays move, since
/// switching them under load would arc their contacts. At the probe current the
/// break is harmless.
async fn change_polarity(
    filament: &dyn FilamentSystem,
    polarity: Polarity,
) -> Result<(), AnyError> {
    debug!("changing the polarity to {:?}", polarity);
    filament.set_output_enabled(false).await?;
    filament.set_polarity(polarity).await?;
    tokio::time::sleep(RELAY_SETTLE_TIME).await;
    filament.set_output_enabled(true).await?;
    Ok(())
}

/// Combines probe samples into resistances, cancelling offsets and thermal
/// EMFs by polarity reversal.
///
/// Each block of consecutive samples in one polarity is averaged to one V, I
/// and t. Every neighbouring pair of blocks in opposite polarities, a and b,
/// gives
///
/// ```text
/// R = |Vₐ − V_b| / (Iₐ + I_b)
/// ```
///
/// Both currents are positive, since the supply's readback doesn't reverse
/// with the relays. One voltage is +R·I + E and the other −R·I + E, where E is
/// the offset, so E cancels in the difference. Which relay state gives the
/// positive voltage depends on how the relays and sense leads are wired, so
/// it's not assumed: the magnitude is the same either way. Each R is
/// timestamped at the midpoint of its two blocks' times. Every block but the
/// first and last is in two pairs, so an offset drifting by ΔE between blocks
/// adds ±ΔE/(Iₐ + I_b), alternating in sign from one pair to the next, which
/// averages out across a fit.
fn probe_resistances(samples: &[ProbeSample]) -> Vec<ProbeResistance> {
    struct Block {
        current: f64,
        polarity: Polarity,
        time: f64,
        voltage: f64,
    }

    let blocks: Vec<Block> = samples
        .chunk_by(|a, b| a.polarity == b.polarity)
        .map(|block| Block {
            current: mean(block.iter().map(|s| s.current_amps)),
            polarity: block[0].polarity,
            time: mean(block.iter().map(|s| s.time_seconds)),
            voltage: mean(block.iter().map(|s| s.voltage_volts)),
        })
        .collect();

    blocks
        .windows(2)
        .filter_map(|pair| {
            let [a, b] = pair else { unreachable!() };
            if !matches!(
                (a.polarity, b.polarity),
                (Polarity::Forward, Polarity::Reverse) | (Polarity::Reverse, Polarity::Forward)
            ) {
                return None;
            }
            Some(ProbeResistance {
                resistance_ohms: (a.voltage - b.voltage).abs() / (a.current + b.current),
                time_seconds: (a.time + b.time) / 2.0,
            })
        })
        .collect()
}

/// Fires the pulse: jumps to `current`, samples until `duration` has passed
/// since then, `maximum_energy` has been delivered, or the supply leaves
/// constant current, then jumps back to the probe current. Returns the energy
/// delivered, in joules.
///
/// Records into `pulse` as it goes, so a run that stops partway still has what
/// was measured.
///
/// The energy is Σ |V|·I·Δt, each sample's power taken over the time since the
/// previous sample, or since the current was set for the first. The last
/// sample's power is taken to continue until the current is set back down.
async fn fire(
    filament: &dyn FilamentSystem,
    current: f64,
    duration: Duration,
    maximum_energy: Option<f64>,
    scale: f64,
    start: Instant,
    pulse: &mut Option<Pulse>,
) -> Result<f64, AnyError> {
    filament.set_heating_current_limit(current).await?;
    let up = Instant::now();
    let deadline = up + duration;
    let pulse = pulse.insert(Pulse {
        down_seconds: None,
        end: None,
        samples: Vec::new(),
        up_seconds: (up - start).as_secs_f64(),
    });

    let mut energy = 0.0;
    let mut power = 0.0;
    let mut previous = up;
    let end = loop {
        let Some(reading) = sample(filament, scale, Some(deadline)).await? else {
            break PulseEnd::Duration;
        };
        let now = Instant::now();

        // A magnitude, because the voltage's sign depends on how the relays
        // and sense leads are wired.
        power = reading.voltage.abs() * reading.current;
        energy += power * (now - previous).as_secs_f64();
        previous = now;

        pulse.samples.push(PulseSample {
            current_amps: reading.current,
            time_seconds: (now - start).as_secs_f64(),
            voltage_volts: reading.voltage,
        });
        println!(
            "  {:>5.2} s: {:.3} A, {:.3} V, {:.3} W, {:.2} J so far",
            (now - up).as_secs_f64(),
            reading.current,
            reading.voltage,
            power,
            energy
        );

        if reading.mode != RegulationMode::ConstantCurrent {
            break PulseEnd::ConstantVoltage;
        }
        if maximum_energy.is_some_and(|maximum| energy >= maximum) {
            break PulseEnd::EnergyLimit;
        }
    };

    filament
        .set_heating_current_limit(PROBE_CURRENT_AMPS)
        .await?;
    let down = Instant::now();
    energy += power * (down - previous).as_secs_f64();

    pulse.down_seconds = Some((down - start).as_secs_f64());
    pulse.end = Some(end);
    Ok(energy)
}

/// The baseline's mean resistance, and its slope against time from a
/// straight-line fit, in ohms and ohms per second, from its probe resistances.
/// `None` with fewer than two.
///
/// The mean is the plain mean of the resistances, R̄. The slope is
/// the ordinary least-squares estimate for R = a + b·t, the b that minimises
/// Σ(Rᵢ − a − b·tᵢ)²:
///
/// ```text
/// b = Σ(tᵢ − t̄)(Rᵢ − R̄) / Σ(tᵢ − t̄)²
/// ```
///
/// i.e. the covariance of t and R over the variance of t, with the 1/n of each
/// cancelling. See "Fitting the regression line" in
/// <https://en.wikipedia.org/wiki/Simple_linear_regression>.
fn baseline_fit(resistances: &[ProbeResistance]) -> Option<(f64, f64)> {
    if resistances.len() < 2 {
        return None;
    }

    let t_mean = mean(resistances.iter().map(|r| r.time_seconds));
    let r_mean = mean(resistances.iter().map(|r| r.resistance_ohms));

    // The numerator and denominator of b.
    let (covariance, variance) = resistances.iter().fold((0.0, 0.0), |(c, v), r| {
        let dt = r.time_seconds - t_mean;
        (c + dt * (r.resistance_ohms - r_mean), v + dt * dt)
    });
    Some((r_mean, covariance / variance))
}

/// The mean of `values`, NaN if there are none.
fn mean(values: impl Iterator<Item = f64>) -> f64 {
    let (sum, n) = values.fold((0.0, 0), |(sum, n), v| (sum + v, n + 1));
    sum / n as f64
}

/// Initialises `env_logger` to log to `out/pulse.log`, keeping the library's
/// logging out of stdout.
fn init_logging() -> Result<(), std::io::Error> {
    fs::create_dir_all("out")?;
    let file = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open("out/pulse.log")?;

    Builder::from_default_env()
        .target(Target::Pipe(Box::new(file)))
        .init();

    info!("Pulse started");
    Ok(())
}

/// The message a panic was raised with, if it was a string.
fn panic_message(panic: &(dyn Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|message| message.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| String::from("no message"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use characteriser::hardware::MockFilamentSystem;

    fn arguments(current: f64, duration: f64, maximum_energy: Option<f64>) -> Arguments {
        Arguments {
            filament_id: String::from("test"),
            current,
            duration,
            maximum_energy,
            device_path: String::new(),
            adc_gauge_number: 1,
            tmp_address: String::new(),
        }
    }

    #[test]
    fn arguments_within_their_limits_are_accepted() {
        assert!(check_arguments(&arguments(2.0, 5.0, None)).is_ok());
        assert!(check_arguments(&arguments(3.0, 10.0, Some(10.0))).is_ok());
    }

    #[test]
    fn arguments_outside_their_limits_are_rejected() {
        for (current, duration, energy) in [
            (PROBE_CURRENT_AMPS, 5.0, None),
            (3.01, 5.0, None),
            (f64::NAN, 5.0, None),
            (2.0, 0.0, None),
            (2.0, 10.01, None),
            (2.0, f64::NAN, None),
            (2.0, 5.0, Some(0.0)),
            (2.0, 5.0, Some(f64::NAN)),
        ] {
            assert!(
                check_arguments(&arguments(current, duration, energy)).is_err(),
                "{} A, {} s, {:?} J",
                current,
                duration,
                energy
            );
        }
    }

    #[test]
    fn the_pressure_must_be_below_the_operating_pressure() {
        assert!(pressure_ok(FILAMENT_OPERATING_PRESSURE_MBAR / 2.0).is_ok());
        assert!(pressure_ok(FILAMENT_OPERATING_PRESSURE_MBAR).is_err());
        assert!(pressure_ok(1e3).is_err());
        assert!(pressure_ok(f64::NAN).is_err());
    }

    #[test]
    fn the_pulse_scale_is_the_smallest_that_fits_the_voltage_limit() {
        // 5 V needs 5 / (4 × 0.8) = 1.56 V/div.
        assert_eq!(pulse_scale(5.0), 2.0);
        assert_eq!(pulse_scale(3.2), 1.0);
    }

    #[test]
    fn the_baseline_fit_finds_the_slope() {
        let resistances: Vec<_> = (0..10)
            .map(|i| ProbeResistance {
                resistance_ohms: 0.1 + 2e-6 * i as f64,
                time_seconds: i as f64,
            })
            .collect();
        let (resistance, slope) = baseline_fit(&resistances).unwrap();
        assert!((resistance - (0.1 + 9e-6)).abs() < 1e-12);
        assert!((slope - 2e-6).abs() < 1e-12);
        assert!(baseline_fit(&resistances[..1]).is_none());
    }

    // An offset common to both polarities cancels, whatever the currents and
    // whichever polarity gives the positive voltage, and each resistance is
    // timed at the midpoint of its two blocks.
    #[test]
    fn reversed_blocks_cancel_an_offset() {
        let resistance = 0.0977;
        let offset = -3e-4;
        for forward_sign in [1.0, -1.0] {
            let mut samples = Vec::new();
            for block in 0..4 {
                let (polarity, sign) = if block % 2 == 0 {
                    (Polarity::Forward, forward_sign)
                } else {
                    (Polarity::Reverse, -forward_sign)
                };
                for i in 0..3 {
                    let current = 0.1 + 1e-4 * block as f64;
                    samples.push(ProbeSample {
                        current_amps: current,
                        polarity,
                        time_seconds: 10.0 * block as f64 + i as f64,
                        voltage_volts: sign * resistance * current + offset,
                    });
                }
            }

            let resistances = probe_resistances(&samples);
            assert_eq!(resistances.len(), 3);
            for (k, r) in resistances.iter().enumerate() {
                assert!((r.resistance_ohms - resistance).abs() < 1e-12, "{:?}", r);
                // Blocks k and k + 1 have mean times 10k + 1 and 10k + 11.
                assert!((r.time_seconds - (10.0 * k as f64 + 6.0)).abs() < 1e-12);
            }
            assert!(probe_resistances(&samples[..3]).is_empty());
        }
    }

    // The probe alternates polarity block by block, starting in the polarity
    // it's given, and the mock's resistance comes back from the pairs. The mock
    // refuses to switch the relays with the output on, so this also checks the
    // output is turned off for each reversal.
    #[tokio::test(start_paused = true)]
    async fn the_probe_reverses_in_blocks() {
        let filament = powered_mock(Polarity::Forward).await;
        filament.set_heating_current_limit(0.1).await.unwrap();
        let mut polarity = Polarity::Forward;
        let mut samples = Vec::new();
        record_probe(
            &filament,
            0.01,
            Duration::from_secs(60),
            Instant::now(),
            &mut polarity,
            &mut samples,
            |_| {},
        )
        .await
        .unwrap();

        let polarities: Vec<_> = samples
            .chunk_by(|a, b| a.polarity == b.polarity)
            .map(|block| block[0].polarity)
            .collect();
        assert_eq!(
            polarities,
            [
                Polarity::Forward,
                Polarity::Reverse,
                Polarity::Forward,
                Polarity::Reverse,
                Polarity::Forward,
                Polarity::Reverse,
            ]
        );
        assert_eq!(polarity, Polarity::Reverse);

        let resistances = probe_resistances(&samples);
        assert_eq!(resistances.len(), 5);
        for r in resistances {
            assert!((r.resistance_ohms - 0.096).abs() < 1e-12, "{:?}", r);
        }
    }

    /// The mock with its output on in `polarity`. Its voltage is positive in
    /// forward polarity and negative in reverse.
    async fn powered_mock(polarity: Polarity) -> MockFilamentSystem {
        let filament = MockFilamentSystem::default();
        filament.set_polarity(polarity).await.unwrap();
        filament.set_output_enabled(true).await.unwrap();
        filament
    }

    // The pulse ends at the deadline, overrunning by no more than one
    // instrument call, and the current goes back to the probe current.
    #[tokio::test(start_paused = true)]
    async fn the_pulse_ends_when_its_duration_has_passed() {
        let filament = powered_mock(Polarity::Forward).await;
        let mut pulse = None;
        let energy = fire(
            &filament,
            2.0,
            Duration::from_secs(2),
            None,
            2.0,
            Instant::now(),
            &mut pulse,
        )
        .await
        .unwrap();

        let pulse = pulse.unwrap();
        assert_eq!(pulse.end, Some(PulseEnd::Duration));
        let length = pulse.down_seconds.unwrap() - pulse.up_seconds;
        assert!((2.0..2.05).contains(&length), "{} s", length);
        assert!(!pulse.samples.is_empty());

        // The mock is 0.096 Ω, so 0.384 W at 2 A.
        assert!((energy - 0.384 * length).abs() < 1e-9, "{} J", energy);
        assert_eq!(
            filament.get_heating_current_limit().await.unwrap(),
            PROBE_CURRENT_AMPS
        );
    }

    // In either polarity: either can read a negative voltage, depending on
    // the wiring, and the energy must still count up.
    #[tokio::test(start_paused = true)]
    async fn the_pulse_ends_early_at_the_maximum_energy() {
        for polarity in [Polarity::Forward, Polarity::Reverse] {
            let filament = powered_mock(polarity).await;
            let mut pulse = None;
            let energy = fire(
                &filament,
                2.0,
                Duration::from_secs(5),
                Some(0.3),
                2.0,
                Instant::now(),
                &mut pulse,
            )
            .await
            .unwrap();

            let pulse = pulse.unwrap();
            assert_eq!(pulse.end, Some(PulseEnd::EnergyLimit), "{:?}", polarity);
            assert!(
                (0.3..0.33).contains(&energy),
                "{:?}: {} J",
                polarity,
                energy
            );
            assert_eq!(
                filament.get_heating_current_limit().await.unwrap(),
                PROBE_CURRENT_AMPS
            );
        }
    }
}
