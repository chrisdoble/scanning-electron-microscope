mod app;
mod constants;
mod hardware;
mod procedure;
mod python;
mod results;
mod steps;
mod style;
mod ui;

use app::{App, AppEvent, Shutdown};
use clap::Parser;
use env_logger::{Builder, Target};
use futures::FutureExt;
use hardware::{
    Hardware, HardwareError, MockFilamentSystem, MockVacuumSystem, RealFilamentSystem,
    RealVacuumSystem,
};
use host::controller::Controller;
use log::*;
use procedure::Context;
use results::Characterisation;
use std::{
    any::Any,
    fs,
    io::Write,
    panic::AssertUnwindSafe,
    process::ExitCode,
    sync::{Arc, Mutex},
    time::Duration,
};
use steps::Section;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

type AnyError = Box<dyn std::error::Error>;

/// How long cleaning up may take before it's given up on.
///
/// Three commands to instruments that each answer in well under a second. It's
/// bounded so that a hung instrument can't stop the terminal being restored.
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the procedure is given to stop once it's been cancelled, before
/// it's aborted.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// How many messages the application's event channel can hold.
///
/// There are two senders, each sending once as it finishes, so this only has to
/// be more than that.
const EVENT_CHANNEL_CAPACITY: usize = 8;

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
            report_error(format_args!("error: {}", e));
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

    // Cancelled as the first step of shutdown, which unblocks whatever the
    // procedure is waiting on.
    let cancel = CancellationToken::new();

    // The step tree. It's shared with the procedure, which mutates it, and the
    // application, which renders it. The lock is a `std::sync::Mutex` because
    // every critical section is a short synchronous mutation or one render
    // pass, and it's never held across an `.await`.
    let root = Arc::new(Mutex::new(Section::default()));

    let characterisation = Characterisation::new();
    let results_path = characterisation.path();
    info!("Writing results to {}", results_path.display());

    let (snapshots_tx, snapshots_rx) = watch::channel(None);
    let (events_tx, events_rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);

    // The procedure waits for the first snapshot before doing anything, so
    // nothing runs against instruments that haven't answered yet. Its result is
    // forwarded to the application; an error sending means it has already
    // gone.
    //
    // A panic is caught and forwarded too. Otherwise the task would die
    // silently and the application would wait for an event that never came.
    let ctx = Context::new(
        cancel.clone(),
        characterisation,
        Arc::clone(&root),
        snapshots_rx.clone(),
    );
    let mut procedure_task = tokio::spawn({
        let events_tx = events_tx.clone();
        let hardware = hardware.clone();
        async move {
            let event = match AssertUnwindSafe(procedure::run(ctx, hardware))
                .catch_unwind()
                .await
            {
                Ok(result) => AppEvent::ProcedureFinished(result),
                Err(panic) => AppEvent::ProcedurePanicked(panic_message(panic.as_ref())),
            };
            let _ = events_tx.send(event).await;
        }
    });

    // The poll task returns only once the hardware has failed for good, which
    // is fatal, so it's forwarded to the application to end the run. An error
    // sending means the application has already gone.
    //
    // It's left running through shutdown. It only reads, and the instruments'
    // locks keep its reads apart from the cleanup's commands.
    tokio::spawn({
        let hardware = hardware.clone();
        async move {
            let e = hardware::poll(hardware, &snapshots_tx).await;
            let _ = events_tx.send(AppEvent::HardwareFailed(e)).await;

            // Only now close the channel. The procedure sees it close as a
            // cancelled wait and reports that too, and reported first it would
            // hide the real cause behind "cancelled".
            drop(snapshots_tx);
        }
    });

    let mut app = App::new(root);
    let result = app.run(&mut terminal, snapshots_rx, events_rx).await;
    match &result {
        Ok(shutdown) => info!("Shutting down: {:?}", shutdown),
        Err(e) => error!("the application failed: {}", e),
    }

    // Everything from here runs however the loop ended, including when `run`
    // itself failed, which is why it's here rather than in the loop. The order
    // matters: the filament is made safe before anything waits on the
    // operator.

    // 1. Cancel, which unblocks whatever the procedure is waiting on.
    cancel.cancel();

    // 2. Wait for the procedure to stop. Cancelling only stops it at its next
    //    cancellation-aware await, and a plain hardware call on the way there,
    //    such as enabling the output, would race the cleanup. Waiting means the
    //    cleanup has the last word. It also lets the procedure's final save
    //    happen. A procedure stuck in a hung instrument call is aborted; any
    //    command still in flight goes through the same instrument lock as the
    //    cleanup's, so the cleanup's still come after.
    match tokio::time::timeout(SHUTDOWN_TIMEOUT, &mut procedure_task).await {
        Ok(_) => info!("The procedure has stopped"),
        Err(_) => {
            error!(
                "the procedure didn't stop within {:?}, aborting it",
                SHUTDOWN_TIMEOUT
            );
            procedure_task.abort();
        }
    }

    // 3. Put the filament system into a safe state. A hung instrument mustn't
    //    stop the terminal being restored, so this is bounded too.
    let cleanup =
        match tokio::time::timeout(CLEANUP_TIMEOUT, hardware.filament.enter_safe_state()).await {
            Ok(Ok(())) => {
                info!("Put the filament system into a safe state");
                Ok(())
            }
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => {
                error!(
                    "putting the filament system into a safe state didn't finish within {:?}",
                    CLEANUP_TIMEOUT
                );
                Err(format!(
                    "cleanup didn't finish within {:?}",
                    CLEANUP_TIMEOUT
                ))
            }
        };

    // Read the TMP's state fresh rather than trusting the last snapshot, which
    // is stale if it's the vacuum side that failed. `None` if it can't be read.
    let tmp_running = match tokio::time::timeout(CLEANUP_TIMEOUT, hardware.vacuum.snapshot()).await
    {
        Ok(Ok(snapshot)) => Some(snapshot.tmp_running),
        Ok(Err(e)) => {
            error!("couldn't read the TMP's state: {}", e);
            None
        }
        Err(_) => {
            error!(
                "reading the TMP's state didn't finish within {:?}",
                CLEANUP_TIMEOUT
            );
            None
        }
    };

    // 4. After a failure, and only once the filament is safe, show what went
    //    wrong until the operator presses a key.
    if let Ok(Shutdown::Error(_)) = &result {
        let acknowledged = async {
            app.show_cleanup(&cleanup)?;
            app.acknowledge(&mut terminal).await
        }
        .await;
        if let Err(e) = acknowledged {
            error!("couldn't show the error: {}", e);
        }
    }

    // 5. Restore the terminal and report.
    //
    // The terminal may be gone by now — that's what SIGHUP means — so nothing
    // from here may panic on a failed write. `ratatui::restore` would report
    // its own failure with `eprintln!`, which panics when stderr is closed.
    if let Err(e) = ratatui::try_restore() {
        error!("couldn't restore the terminal: {}", e);
    }
    info!("Exiting");

    // The results are only written once the procedure has started, so there
    // may be nothing to point at.
    if results_path.exists() {
        report(format_args!(
            "Results written to {}",
            results_path.display()
        ));
    }
    if let Err(e) = &cleanup {
        report_error(format_args!(
            "Warning: the filament system may still be powered: {}",
            e
        ));
    }

    // After a completed run the procedure has already stopped the TMP.
    match tmp_running {
        Some(false) => {}
        Some(true) => report(format_args!(
            "The TMP is still running. Use the vacuum control binary to stop it."
        )),
        None => report(format_args!(
            "Couldn't read the TMP's state, so it may still be running. Check it, and use the \
             vacuum control binary to stop it."
        )),
    }

    match result? {
        Shutdown::Normal => Ok(()),
        Shutdown::Error(message) | Shutdown::Panicked(message) => Err(message.into()),
    }
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

/// The message a panic was raised with, if it was a string.
fn panic_message(panic: &(dyn Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|message| message.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| String::from("no message"))
}

/// Prints a line for the operator once the terminal has been restored.
///
/// Unlike `println!` it doesn't panic if the write fails, which it does when
/// the terminal has been closed. There's nobody to tell by then anyway.
fn report(line: std::fmt::Arguments) {
    let _ = writeln!(std::io::stdout(), "{}", line);
}

/// As `report`, but for errors and warnings, on stderr.
fn report_error(line: std::fmt::Arguments) {
    let _ = writeln!(std::io::stderr(), "{}", line);
}
