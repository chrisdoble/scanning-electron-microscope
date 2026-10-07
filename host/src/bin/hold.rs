/// Holds the filament at a high current and then a low one, logging its
/// resistance about once a second, to show whether its mount slowly warms and
/// cools.
///
/// A slow creep in resistance at 300 mA that reverses after stepping down to
/// 100 mA can only be thermal, whereas instrument drift wouldn't care which way
/// the current went. Start with the filament having rested unpowered for 20–30
/// minutes, so its mount is at the flange's temperature.
///
/// IMPORTANT: this has none of the characteriser's interlocks. Pump the chamber
/// down and start the TMP first, and keep it running throughout.
///
/// Writes `out/hold-<start time>.csv` with one row per sample:
/// `seconds,setpoint_amps,volts,amps,ohms`. `volts` is the scope's average over
/// one 20 ms acquisition in a single polarity, so it includes the scope's
/// offset, which is constant and doesn't affect drift. Ctrl+C stops it early,
/// keeping what's been logged.
use clap::Parser;
use host::{
    controller::Controller,
    oscilloscope::{Oscilloscope, Voltages},
    power_supply::{Polarity, PowerSupply},
};
use std::{
    fs::{self, File},
    io::Write,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

type AnyError = Box<dyn std::error::Error>;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), AnyError> {
    let args = Arguments::parse();
    let controller = Arc::new(Controller::new(&args.device_path)?);
    let power_supply = PowerSupply::new(controller).await?;
    let oscilloscope = Oscilloscope::new().await?;

    let started = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    fs::create_dir_all("out")?;
    let path = format!("out/hold-{}.csv", started);
    let mut csv = File::create(&path)?;
    writeln!(csv, "seconds,setpoint_amps,volts,amps,ohms")?;
    println!("Logging to {}", path);

    let result = tokio::select! {
        result = hold(&power_supply, &oscilloscope, &mut csv) => result,
        _ = tokio::signal::ctrl_c() => {
            println!("Interrupted");
            Ok(())
        }
    };

    // However it ended: output off, current and voltage zero, relays
    // de-energised. Stepping straight to 0 A is harmless at these currents.
    let reset = power_supply.reset().await;
    println!("Turned the output off");
    result?;
    reset?;
    Ok(())
}

/// Ramps to `HIGH_AMPS` and holds it, steps down to `LOW_AMPS` and holds that,
/// then ramps to 0 A, logging every sample to `csv`.
async fn hold(
    power_supply: &PowerSupply,
    oscilloscope: &Oscilloscope,
    csv: &mut File,
) -> Result<(), AnyError> {
    const HIGH_AMPS: f64 = 0.3;
    const LOW_AMPS: f64 = 0.1;
    const HOLD_TIME: Duration = Duration::from_secs(15 * 60);

    // Smallest first; the first that doesn't clip at `HIGH_AMPS` is used for
    // the whole hold.
    const SCALES_VOLTS_PER_DIVISION: [f64; 7] = [0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0];

    println!("Resetting the power supply and the oscilloscope");
    power_supply.reset().await?;
    oscilloscope.reset().await?;
    power_supply.set_voltage_limit(2.0).await?;
    power_supply
        .set_overcurrent_protection(HIGH_AMPS + 0.05)
        .await?;
    power_supply.set_polarity(Polarity::Forward).await?;
    power_supply.set_output_enabled(true).await?;
    println!("Enabled the output");

    ramp(power_supply, 0.0, HIGH_AMPS).await?;

    println!("Choosing the vertical scale");
    let mut scale = None;
    for candidate in SCALES_VOLTS_PER_DIVISION {
        println!("  trying {} V/div", candidate);
        oscilloscope.set_vertical_scale(candidate).await?;
        if average(oscilloscope.get_voltages().await?, candidate).is_some() {
            scale = Some(candidate);
            break;
        }
    }
    let scale = scale.ok_or("the filament voltage clips at every scale")?;
    println!("Using {} V/div", scale);

    let start = Instant::now();
    log_for(
        power_supply,
        oscilloscope,
        csv,
        start,
        scale,
        1,
        HIGH_AMPS,
        HOLD_TIME,
    )
    .await?;
    ramp(power_supply, HIGH_AMPS, LOW_AMPS).await?;
    log_for(
        power_supply,
        oscilloscope,
        csv,
        start,
        scale,
        2,
        LOW_AMPS,
        HOLD_TIME,
    )
    .await?;
    ramp(power_supply, LOW_AMPS, 0.0).await?;
    println!("Done after {}", minutes(start.elapsed()));
    Ok(())
}

/// Samples continuously at `setpoint` for `duration`, writing a row per sample
/// with the time since `start`, and printing the progress and resistance every
/// half minute. This is hold `number` of the two, which take `2 × duration`
/// between them.
#[expect(clippy::too_many_arguments)]
async fn log_for(
    power_supply: &PowerSupply,
    oscilloscope: &Oscilloscope,
    csv: &mut File,
    start: Instant,
    scale: f64,
    number: u32,
    setpoint: f64,
    duration: Duration,
) -> Result<(), AnyError> {
    const PRINT_INTERVAL: Duration = Duration::from_secs(30);

    println!(
        "Hold {} of 2: {:.0} mA for {}",
        number,
        setpoint * 1000.0,
        minutes(duration)
    );

    let phase_start = Instant::now();
    let mut last_print: Option<Instant> = None;
    while phase_start.elapsed() < duration {
        let voltages = oscilloscope.get_voltages().await?;
        let amps = power_supply.get_current().await?;
        let seconds = start.elapsed().as_secs_f64();

        // A clipped reading is logged as empty rather than ending the hold.
        let (volts, ohms) = match average(voltages, scale) {
            Some(volts) => (volts.to_string(), (volts / amps).to_string()),
            None => {
                eprintln!("{:.1} s: the voltage clipped", seconds);
                (String::new(), String::new())
            }
        };
        writeln!(
            csv,
            "{:.3},{},{},{},{}",
            seconds, setpoint, volts, amps, ohms
        )?;

        if last_print.is_none_or(|time| time.elapsed() >= PRINT_INTERVAL) {
            let overall = start.elapsed().as_secs_f64() / (2.0 * duration.as_secs_f64());
            println!(
                "  [{} of 2] {} of {} ({:.0}% overall)  R = {} Ω",
                number,
                minutes(phase_start.elapsed()),
                minutes(duration),
                overall * 100.0,
                ohms
            );
            last_print = Some(Instant::now());
        }
    }
    Ok(())
}

/// Ramps the current from `from` to `to` in 5 mA steps every 100 ms, as the
/// characteriser does.
async fn ramp(power_supply: &PowerSupply, from: f64, to: f64) -> Result<(), AnyError> {
    const STEP_AMPS: f64 = 0.005;
    const STEP_INTERVAL: Duration = Duration::from_millis(100);

    println!(
        "Ramping from {:.0} mA to {:.0} mA",
        from * 1000.0,
        to * 1000.0
    );

    let steps = ((to - from).abs() / STEP_AMPS).round().max(1.0) as usize;
    for step in 1..=steps {
        power_supply
            .set_current_limit(from + (to - from) * step as f64 / steps as f64)
            .await?;
        tokio::time::sleep(STEP_INTERVAL).await;
    }

    // The supply can take about 0.6 s to apply a setpoint.
    tokio::time::sleep(Duration::from_secs(1)).await;
    Ok(())
}

/// `duration` as minutes and seconds, e.g. `2:05`.
fn minutes(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// The acquisition's average, or `None` if it's clipped on `scale`: any value
/// invalid, or the peak beyond 80% of the way to the edge of the screen, as in
/// the characteriser.
fn average(voltages: Voltages, scale: f64) -> Option<f64> {
    let limit = 0.8 * 4.0 * scale;
    match voltages {
        Voltages {
            average: Some(average),
            maximum: Some(maximum),
            minimum: Some(minimum),
        } if maximum.abs().max(minimum.abs()) <= limit => Some(average),
        _ => None,
    }
}

#[derive(Parser)]
struct Arguments {
    /// The path to the controller device, e.g. /dev/tty.usbmodem11201.
    device_path: String,
}
