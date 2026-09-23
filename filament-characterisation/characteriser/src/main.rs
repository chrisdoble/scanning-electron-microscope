mod app;
mod constants;
mod hardware;
mod python;
mod steps;
mod style;
mod ui;

use app::App;
use clap::Parser;
use env_logger::{Builder, Target};
use hardware::{
    Hardware, HardwareError, MockFilamentSystem, MockVacuumSystem, RealFilamentSystem,
    RealVacuumSystem,
};
use host::controller::Controller;
use log::*;
use std::{
    fs,
    process::ExitCode,
    sync::{Arc, Mutex},
};

type AnyError = Box<dyn std::error::Error>;

/// Reports `run`'s result and turns it into an exit status.
///
/// `main` doesn't return the `Result` itself because Rust would then report it
/// with `Debug`, which for these errors drops the `Display` message — and the
/// `Display` of a `PythonError::Environment` is what tells the operator which
/// commands to run.
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

/// Runs the application.
async fn run() -> Result<(), AnyError> {
    init_logging()?;

    let args = Arguments::parse();

    // Check the Python environment before taking over the terminal, so the
    // operator sees the error and how to fix it.
    python::check_environment().await?;

    let hardware = build_hardware(&args).await?;

    // TODO: the poll task takes these every `POLL_INTERVAL` and publishes them
    // on a watch channel (step 6 of the design document's build order). Until it
    // exists, one of each proves the adapters work.
    debug!("{:?}", hardware.vacuum.snapshot().await?);
    debug!("{:?}", hardware.filament.snapshot().await?);

    // `ratatui::init` installs a panic hook that restores the terminal before
    // chaining to the previous hook, so a panic message isn't swallowed by the
    // alternate screen. Anything that wants to run before that restore has to
    // install its own hook *before* this call.
    //
    // Note that a panic hook can't run the asynchronous cleanup that puts the
    // filament system into a safe state — hooks are synchronous. That's why the
    // procedure task must never panic, and why every fallible call inside it
    // returns an error instead.
    let mut terminal = ratatui::init();

    // TODO: this is `steps::demo()` until the procedure builds the tree (step 8
    // of the design document's build order), at which point it becomes
    // `Section::default()`.
    let root = Arc::new(Mutex::new(steps::demo()));

    // TODO: the procedure task replaces this (step 8 of the design document's
    // build order).
    tokio::spawn(steps::run_demo(Arc::clone(&root)));

    // Use an `async` block to ensure we call `ratatui::restore` on error.
    let result: Result<(), AnyError> = async { App::new(root).run(&mut terminal).await }.await;

    ratatui::restore();

    result
}

/// Builds the hardware the application talks to.
///
/// `--mock` mocks both subsystems. The two are separate `Arc`s so that one can
/// be mocked without the other, which is a change here rather than a new flag.
async fn build_hardware(args: &Arguments) -> Result<Hardware, HardwareError> {
    if args.mock {
        info!("Running against mocked hardware");
        return Ok(Hardware {
            filament: Arc::new(MockFilamentSystem::default()),
            vacuum: Arc::new(MockVacuumSystem::default()),
        });
    }

    info!("Running against real hardware");

    // Present because clap's `required_unless_present = "mock"` on each makes
    // them so, and this branch is only reached without `--mock`.
    let device_path = args.device_path.as_deref().expect("missing device path");
    let adc_gauge_number = args.adc_gauge_number.expect("missing ADC gauge number");
    let tmp_address = args.tmp_address.as_deref().expect("missing TMP address");

    // One controller, shared by the ADC, the TMP and the polarity relays.
    let controller = Arc::new(Controller::new(device_path).map_err(|source| {
        error!(
            "failed to open the controller at {}: {}",
            device_path, source
        );
        HardwareError::Controller {
            path: device_path.to_string(),
            source,
        }
    })?);

    Ok(Hardware {
        filament: Arc::new(RealFilamentSystem::new(Arc::clone(&controller)).await?),
        vacuum: Arc::new(RealVacuumSystem::new(controller, adc_gauge_number, tmp_address).await?),
    })
}

/// Initialise `env_logger` to log to `out/app.log`.
fn init_logging() -> Result<(), std::io::Error> {
    fs::create_dir_all("out")?;
    let file = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open("out/app.log")?;

    Builder::from_default_env()
        .target(Target::Pipe(Box::new(file)))
        .init();

    info!("Application started");
    Ok(())
}

#[derive(Parser)]
struct Arguments {
    /// The path to the controller device, e.g. /dev/tty.usbmodem11201.
    #[arg(required_unless_present = "mock")]
    device_path: Option<String>,

    /// The gauge number to use when communicating with the ADC, e.g. 1.
    #[arg(
        required_unless_present = "mock",
        value_parser = clap::value_parser!(u8).range(1..=2)
    )]
    adc_gauge_number: Option<u8>,

    /// The address of the TMP in the RS-485 Pfeiffer Vacuum Protocol, e.g. 001.
    ///
    /// This corresponds to TMP parameter 797.
    #[arg(required_unless_present = "mock")]
    tmp_address: Option<String>,

    /// Run against simulated hardware instead of the real rig.
    #[arg(long)]
    mock: bool,
}
