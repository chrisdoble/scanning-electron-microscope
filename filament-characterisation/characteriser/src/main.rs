mod app;
mod python;
mod ui;

use app::App;
use clap::Parser;
use env_logger::{Builder, Target};
use log::*;
use std::{fs, process::ExitCode};

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
    if args.mock {
        info!("Running against mocked hardware");
    } else {
        info!("Running against real hardware");
    }

    // Check the Python environment before taking over the terminal, so the
    // operator sees the error and how to fix it.
    python::check_environment().await?;

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

    // Use an `async` block to ensure we call `ratatui::restore` on error.
    let result: Result<(), AnyError> = async { App::new().run(&mut terminal).await }.await;

    ratatui::restore();

    result
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

// TODO: remove this once `build_hardware` reads the positional arguments.
#[allow(dead_code)]
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
