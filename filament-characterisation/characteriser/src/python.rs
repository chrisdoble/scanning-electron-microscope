//! Runs the analysis scripts in the crate's `python` directory.
//!
//! The scripts need a virtual environment beside them, which the operator
//! creates once with `SETUP_COMMANDS` below. `check_environment` is called at
//! start-up so a missing or broken environment is reported before the
//! application takes over the terminal.
//!
//! Every script reads one JSON object on stdin and writes one on stdout. The
//! Rust types here are the source of truth for their shapes: each derives
//! `JsonSchema`, and the schemas generated from them are committed in
//! `python/schemas/`, where the scripts validate their input against them. The
//! `schemas_are_up_to_date` test fails if the two drift apart.

use crate::results::{
    ColdResistanceAnalysis, ColdResistanceFitParameters, ColdResistancePointAnalysis, Derived,
    Measurement,
};
use log::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{io::ErrorKind, path::Path, process::Stdio, time::Duration};
use thiserror::Error;
use tokio::{io::AsyncWriteExt, process::Command};

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

    /// A script's input couldn't be serialised.
    #[error("couldn't serialise the input to {script}: {source}")]
    InvalidInput {
        script: String,
        #[source]
        source: serde_json::Error,
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

/// The input to `mean_and_standard_error.py`.
#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeanAndStandardErrorInput {
    /// The samples, at least two of them.
    pub samples: Vec<f64>,
}

/// The output of `mean_and_standard_error.py`.
#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeanAndStandardErrorOutput {
    /// The standard error of the mean.
    pub uncertainty: f64,

    /// The mean of the samples.
    pub value: f64,
}

/// The input to `cold_resistance_point.py`: one setpoint's measurements in
/// both polarities.
#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ColdResistancePointInput {
    /// The supply's current readback resolution in amperes, for the
    /// quantisation floor.
    pub current_resolution_amps: f64,

    /// The current in the polarity that gives a negative voltage.
    pub negative_current_amps: Measurement,

    /// The voltage in the polarity that gives a negative voltage.
    pub negative_voltage_volts: Measurement,

    /// How large the voltage that doesn't reverse with the current may be
    /// before it's warned about, in volts.
    pub offset_warning_volts: f64,

    /// The current in the polarity that gives a positive voltage.
    pub positive_current_amps: Measurement,

    /// The voltage in the polarity that gives a positive voltage.
    pub positive_voltage_volts: Measurement,
}

/// The input to `cold_resistance_fit.py`.
#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ColdResistanceFitInput {
    /// The fit's scalar inputs.
    pub parameters: ColdResistanceFitParameters,

    /// Every setpoint's reversal-corrected voltage and current.
    pub points: Vec<ColdResistanceFitPoint>,
}

/// One setpoint's input to the fit, from its `ColdResistancePointAnalysis`.
#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ColdResistanceFitPoint {
    /// The mean of the two polarities' currents, with the quantisation floor.
    pub current_amps: Derived,

    /// Half the difference between the two polarities' voltages.
    pub voltage_volts: Derived,
}

/// Checks that the virtual environment exists and has the packages the scripts
/// import.
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

    // Importing the packages is the only way to tell a complete environment
    // from one whose `pip install` never ran.
    let import = "import jsonschema, numpy, scipy, uncertainties";
    debug!("running {} -c \"{}\"", interpreter.display(), import);
    let output = run(
        &interpreter,
        &["-c".to_string(), import.to_string()],
        &format!("-c \"{}\"", import),
        &[],
    )
    .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        error!("couldn't import the required packages: {}", stderr);
        return Err(PythonError::Environment(format!(
            "{} can't import the required packages: {}",
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
pub async fn mean_and_standard_error(samples: &[f64]) -> Result<(f64, f64), PythonError> {
    let output: MeanAndStandardErrorOutput = run_script(
        "mean_and_standard_error.py",
        &MeanAndStandardErrorInput {
            samples: samples.to_vec(),
        },
    )
    .await?;
    Ok((output.value, output.uncertainty))
}

/// Analyses one setpoint: reverses out what doesn't change sign with the
/// current, and derives the resistance and its uncertainty.
///
/// Wraps `cold_resistance_point.py`.
// Used from commit 8 of COLD_RESISTANCE.md's implementation order.
#[expect(dead_code)]
pub async fn cold_resistance_point(
    input: &ColdResistancePointInput,
) -> Result<ColdResistancePointAnalysis, PythonError> {
    run_script("cold_resistance_point.py", input).await
}

/// Fits the setpoints' resistances against the square of their currents,
/// extrapolates to zero current, and works out the uncertainty budget.
///
/// Wraps `cold_resistance_fit.py`.
// Used from commit 9 of COLD_RESISTANCE.md's implementation order.
#[expect(dead_code)]
pub async fn cold_resistance_fit(
    input: &ColdResistanceFitInput,
) -> Result<ColdResistanceAnalysis, PythonError> {
    run_script("cold_resistance_fit.py", input).await
}

/// Runs a script from the crate's `python` directory.
///
/// `input` is written to the script's stdin as JSON. The script is expected to
/// print a single JSON object on stdout, which is deserialised into `O`.
/// Anything it prints on stderr is logged.
pub async fn run_script<I: Serialize, O: DeserializeOwned>(
    script: &str,
    input: &I,
) -> Result<O, PythonError> {
    let python_dir = Path::new(PYTHON_DIR);
    let interpreter = python_dir.join(VENV_PYTHON);
    let script_path = python_dir.join(script).display().to_string();

    let input = serde_json::to_vec(input).map_err(|source| {
        error!("couldn't serialise the input to {}: {}", script, source);
        PythonError::InvalidInput {
            script: script.to_string(),
            source,
        }
    })?;

    debug!(
        "running {} {} with {}",
        interpreter.display(),
        script_path,
        String::from_utf8_lossy(&input)
    );
    let output = run(&interpreter, &[script_path], script, &input).await?;

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

/// Runs `interpreter` with `args`, writing `input` to its stdin and capturing
/// its output.
///
/// `script` names what's being run, for errors. Never goes through a shell.
async fn run(
    interpreter: &Path,
    args: &[String],
    script: &str,
    input: &[u8],
) -> Result<std::process::Output, PythonError> {
    let child = async {
        let mut child = Command::new(interpreter)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let mut stdin = child.stdin.take().expect("stdin is piped");

        // A script that exits before reading its input, e.g. because an import
        // failed, closes the pipe. That isn't the error worth reporting: its
        // exit status and stderr are, so carry on and collect them.
        if let Err(e) = stdin.write_all(input).await
            && e.kind() != ErrorKind::BrokenPipe
        {
            return Err(e);
        }

        // Close the pipe before waiting. Otherwise the script never sees the
        // end of its input and blocks until the timeout.
        drop(stdin);

        child.wait_with_output().await
    };

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

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::{Schema, schema_for};
    use std::fs;

    /// Checks that the schemas committed in `python/schemas/` match the Rust
    /// types, so a change to a type can't silently diverge from what the
    /// scripts expect.
    ///
    /// Run with `UPDATE_SCHEMAS=1` to write them instead.
    #[test]
    fn schemas_are_up_to_date() {
        // Each script, by the name its schemas are filed under, with its input
        // and output schemas.
        let scripts: [(&str, Schema, Schema); 3] = [
            (
                "cold_resistance_fit",
                schema_for!(ColdResistanceFitInput),
                schema_for!(ColdResistanceAnalysis),
            ),
            (
                "cold_resistance_point",
                schema_for!(ColdResistancePointInput),
                schema_for!(ColdResistancePointAnalysis),
            ),
            (
                "mean_and_standard_error",
                schema_for!(MeanAndStandardErrorInput),
                schema_for!(MeanAndStandardErrorOutput),
            ),
        ];

        let update = std::env::var_os("UPDATE_SCHEMAS").is_some();
        let schemas_dir = Path::new(PYTHON_DIR).join("schemas");
        if update {
            fs::create_dir_all(&schemas_dir).unwrap();
        }

        for (script, input, output) in scripts {
            for (kind, schema) in [("input", input), ("output", output)] {
                let path = schemas_dir.join(format!("{}.{}.json", script, kind));
                let expected = serde_json::to_string_pretty(&schema).unwrap() + "\n";

                if update {
                    fs::write(&path, expected).unwrap();
                } else {
                    let actual = fs::read_to_string(&path).unwrap_or_default();
                    assert_eq!(
                        actual,
                        expected,
                        "{} is out of date; run the test with UPDATE_SCHEMAS=1",
                        path.display()
                    );
                }
            }
        }
    }
}
