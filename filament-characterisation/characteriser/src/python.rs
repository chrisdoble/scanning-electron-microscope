//! Runs the analysis scripts in the crate's `python` directory.
//!
//! The scripts need a virtual environment beside them, which the operator
//! creates once with `SETUP_COMMANDS` below. `check_environment` is called at
//! start-up so a missing or broken environment is reported before the
//! application takes over the terminal.

use log::*;
use serde::{Deserialize, de::DeserializeOwned};
use std::{path::Path, process::Stdio, time::Duration};
use thiserror::Error;
use tokio::process::Command;

/// The directory holding the analysis scripts and their virtual environment.
///
/// Resolved at compile time from the crate root, since the binary is run from
/// the source tree.
const PYTHON_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/python");

/// How long a Python process may run before it's killed.
///
/// Starting the interpreter and importing `uncertainties` takes about 40 ms,
/// and the scripts are small statistics calculations on top of that, so this is
/// two orders of magnitude more than anything here should need. It's only here
/// to stop a process that hangs from hanging the application with it.
const PYTHON_TIMEOUT: Duration = Duration::from_secs(5);

/// The commands that create the virtual environment, run from the crate
/// directory.
///
/// Reported as part of `PythonError::Environment` so the operator can fix a
/// broken environment without reading the source.
const SETUP_COMMANDS: &str =
    "    python3 -m venv python/.venv\n    python/.venv/bin/pip install -r python/requirements.txt";

/// The interpreter inside the virtual environment.
const VENV_PYTHON: &str = ".venv/bin/python";

/// An error encountered while running one of the analysis scripts.
#[derive(Debug, Error)]
pub enum PythonError {
    /// The virtual environment is missing or unusable.
    #[error("{0}\n\ncreate the virtual environment with:\n\n{SETUP_COMMANDS}")]
    Environment(String),

    /// A script exited with a non-zero status.
    #[error("{script} exited with {status}: {stderr}")]
    Failed {
        script: String,
        status: String,
        stderr: String,
    },

    /// A script's output couldn't be deserialised.
    #[error("couldn't parse the output of {script}: {source}: {stderr}")]
    InvalidOutput {
        script: String,
        #[source]
        source: serde_json::Error,
        stderr: String,
    },

    /// A script couldn't be run at all.
    #[error("couldn't run {script}: {source}")]
    NotRun {
        script: String,
        #[source]
        source: std::io::Error,
    },

    /// A script didn't finish within `PYTHON_TIMEOUT`.
    #[error("{script} didn't finish within {PYTHON_TIMEOUT:?}")]
    TimedOut { script: String },
}

/// Checks that the virtual environment exists and has `uncertainties` in it.
///
/// Call this before `ratatui::init` so a failure prints normally rather than
/// into the alternate screen.
pub async fn check_environment() -> Result<(), PythonError> {
    let interpreter = Path::new(PYTHON_DIR).join(VENV_PYTHON);

    if !interpreter.exists() {
        error!("no interpreter at {}", interpreter.display());
        return Err(PythonError::Environment(format!(
            "there's no interpreter at {}",
            interpreter.display()
        )));
    }

    // Importing the package is the only way to tell a complete environment
    // from one whose `pip install` never ran.
    let script = "-c \"import uncertainties\"";
    debug!(
        "running {} -c \"import uncertainties\"",
        interpreter.display()
    );
    let output = run(
        &interpreter,
        &["-c".to_string(), "import uncertainties".to_string()],
        script,
    )
    .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        error!("couldn't import uncertainties: {}", stderr);
        return Err(PythonError::Environment(format!(
            "{} can't import uncertainties: {}",
            interpreter.display(),
            stderr
        )));
    }

    info!("using the Python interpreter at {}", interpreter.display());
    Ok(())
}

/// The mean and standard error of a set of samples.
///
/// Wraps `mean_and_standard_error.py`, which is the calculation behind every
/// measurement.
// TODO: remove this once the procedure records measurements.
#[allow(dead_code)]
pub async fn mean_and_standard_error(samples: &[f64]) -> Result<(f64, f64), PythonError> {
    /// The object `mean_and_standard_error.py` prints.
    #[derive(Deserialize)]
    struct Output {
        uncertainty: f64,
        value: f64,
    }

    let args: Vec<String> = samples.iter().map(f64::to_string).collect();
    let output: Output = run_script("mean_and_standard_error.py", &args).await?;
    Ok((output.value, output.uncertainty))
}

/// Runs a script from the crate's `python` directory.
///
/// `args` are passed positionally. The script is expected to print a single
/// JSON object on stdout, which is deserialised into `T`. Anything it prints on
/// stderr is logged.
// TODO: remove this once the procedure records measurements.
#[allow(dead_code)]
pub async fn run_script<T: DeserializeOwned>(
    script: &str,
    args: &[String],
) -> Result<T, PythonError> {
    let python_dir = Path::new(PYTHON_DIR);
    let interpreter = python_dir.join(VENV_PYTHON);

    let mut script_args = Vec::with_capacity(args.len() + 1);
    script_args.push(python_dir.join(script).display().to_string());
    script_args.extend_from_slice(args);

    debug!(
        "running {} {}",
        interpreter.display(),
        script_args.join(" ")
    );
    let output = run(&interpreter, &script_args, script).await?;

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        debug!("{} wrote to stderr: {}", script, stderr);
    }

    if !output.status.success() {
        error!("{} exited with {}: {}", script, output.status, stderr);
        return Err(PythonError::Failed {
            script: script.to_string(),
            status: output.status.to_string(),
            stderr,
        });
    }

    serde_json::from_slice(&output.stdout).map_err(|source| {
        error!("couldn't parse the output of {}: {}", script, source);
        PythonError::InvalidOutput {
            script: script.to_string(),
            source,
            stderr,
        }
    })
}

/// Runs `interpreter` with `args`, capturing its output.
///
/// `script` names what's being run, for errors. Never goes through a shell.
async fn run(
    interpreter: &Path,
    args: &[String],
    script: &str,
) -> Result<std::process::Output, PythonError> {
    let child = Command::new(interpreter)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output();

    match tokio::time::timeout(PYTHON_TIMEOUT, child).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(source)) => {
            error!("couldn't run {}: {}", script, source);
            Err(PythonError::NotRun {
                script: script.to_string(),
                source,
            })
        }

        // The future is dropped here, and `kill_on_drop` means dropping it
        // kills the process rather than leaving it running.
        Err(_) => {
            error!("{} didn't finish within {:?}", script, PYTHON_TIMEOUT);
            Err(PythonError::TimedOut {
                script: script.to_string(),
            })
        }
    }
}
