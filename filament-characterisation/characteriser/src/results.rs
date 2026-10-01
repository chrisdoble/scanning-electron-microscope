//! What a characterisation run measures, and how it's written to disk.

use host::power_supply::Polarity;
use log::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;

/// An error encountered while writing the results to disk.
#[derive(Debug, Error)]
pub enum ResultsError {
    /// The file, or the directory it goes in, couldn't be written.
    #[error("couldn't write the results to {}: {source}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The results couldn't be serialised.
    #[error("couldn't serialise the results: {0}")]
    Serialise(#[from] serde_json::Error),
}

/// A measured quantity: the samples taken, and the value derived from them.
///
/// `value` and `uncertainty` are the mean and standard error returned by
/// `mean_and_standard_error.py`, not calculated in Rust.
///
/// This is the shape every measured field takes, and the unit goes in the name
/// of the field holding it — `filament_voltage_volts: Measurement` — since the
/// consumer is another program with no doc comments to read.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Measurement {
    /// The individual samples, in the order they were taken.
    pub samples: Vec<f64>,

    /// The standard error of the mean.
    pub uncertainty: f64,

    /// The mean of the samples.
    pub value: f64,
}

/// A value derived by Python from measurements, with its standard uncertainty.
///
/// Unlike `Measurement` there are no samples: it's calculated from other
/// quantities, not sampled.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Derived {
    /// The standard uncertainty.
    pub uncertainty: f64,

    /// The value.
    pub value: f64,
}

/// The fit's scalar inputs.
///
/// With the points, this is everything `cold_resistance_fit.py` needs, so the
/// fit can be re-run from the results file alone, and anyone reading the file
/// can see which assumptions produced the result.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ColdResistanceFitParameters {
    /// The bound on the supply's current readback gain error, relative, after
    /// widening for the room temperature.
    pub current_gain_bound: f64,

    /// The bound on the supply's current readback offset in amperes, after
    /// widening for the room temperature.
    pub current_offset_bound_amps: f64,

    /// The bound on the filament temperature in kelvin, $a_T$: the
    /// thermometer's bound plus half the drift over the run.
    pub filament_temperature_bound_kelvin: f64,

    /// The filament temperature, $T_f$: the mean of the start and end chamber
    /// temperatures.
    pub filament_temperature_celsius: f64,

    /// The temperature the resistance is corrected to.
    pub reference_temperature_celsius: f64,

    /// The bound on tungsten's temperature coefficient of resistance, $a_\alpha$.
    pub temperature_coefficient_bound_per_kelvin: f64,

    /// Tungsten's temperature coefficient of resistance near room
    /// temperature, $\alpha$.
    pub temperature_coefficient_per_kelvin: f64,

    /// The bound on the oscilloscope's gain error, relative.
    pub voltage_gain_bound: f64,
}

/// The analysis of one setpoint: exactly the output of
/// `cold_resistance_point.py`.
#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ColdResistancePointAnalysis {
    /// The mean of the two polarities' currents, with the quantisation floor.
    pub current_amps: Derived,

    /// The square of `current_amps`, the fit's $x$.
    pub current_squared_amps_squared: Derived,

    /// Half the sum of the two polarities' voltages: what didn't reverse with
    /// the current. A diagnostic.
    pub offset_voltage_volts: Derived,

    /// `voltage_volts` / `current_amps`.
    pub resistance_ohms: Derived,

    /// Half the difference between the two polarities' voltages, so that
    /// anything that doesn't reverse with the current cancels.
    pub voltage_volts: Derived,

    /// Warnings raised by the analysis, e.g. a large offset voltage.
    pub warnings: Vec<String>,
}

/// The fit and uncertainty budget: exactly the output of
/// `cold_resistance_fit.py`.
///
/// Every uncertainty is a standard uncertainty. Expanded uncertainties aren't
/// stored: they're twice these, and are computed for display.
#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ColdResistanceAnalysis {
    /// The probability of a chi-squared at least this large if the line and
    /// the per-point uncertainties are right.
    pub chi_squared_p_value: f64,

    /// The contribution of the supply's readback gain error.
    pub current_gain_uncertainty_ohms: f64,

    /// The contribution of the supply's readback offset error, from the corner
    /// analysis.
    pub current_offset_uncertainty_ohms: f64,

    /// The statistical uncertainty of the fit's intercept, inflated by the
    /// Birge ratio when that's above 1.
    pub fit_uncertainty_ohms: f64,

    /// The fitted intercept with the readback offset at its lower and upper
    /// bounds, in that order.
    pub offset_corner_resistances_ohms: [f64; 2],

    /// Chi-squared divided by its degrees of freedom.
    pub reduced_chi_squared: f64,

    /// The cold resistance corrected to the reference temperature, $R_{20}$,
    /// with its combined standard uncertainty.
    pub reference_resistance_ohms: Derived,

    /// The cold resistance at the filament temperature, $R_0$: the fit's
    /// intercept, with its combined standard uncertainty. The other
    /// `*_uncertainty_ohms` fields are that uncertainty's components.
    pub resistance_ohms: Derived,

    /// The fit's slope, $b$, which measures how strongly the filament heats
    /// itself.
    pub slope_ohms_per_amp_squared: Derived,

    /// The contribution of the uncertainty in tungsten's temperature
    /// coefficient to `reference_resistance_ohms`' uncertainty.
    pub temperature_coefficient_uncertainty_ohms: f64,

    /// The contribution of the uncertainty in the filament temperature to
    /// `reference_resistance_ohms`' uncertainty.
    pub temperature_uncertainty_ohms: f64,

    /// The contribution of the oscilloscope's gain error.
    pub voltage_gain_uncertainty_ohms: f64,

    /// Warnings raised by the analysis, e.g. a poor fit.
    pub warnings: Vec<String>,
}

/// Everything recorded while measuring the cold resistance.
///
/// Created once both temperatures are known and saved at every step after, so
/// the `Option` fields are exactly the ones not yet known at those saves, and
/// an aborted run still leaves everything measured up to that point on disk.
#[derive(Debug, Deserialize, Serialize)]
pub struct ColdResistance {
    /// The fit and uncertainty budget, once every setpoint has been measured.
    pub analysis: Option<ColdResistanceAnalysis>,

    /// The operator-entered chamber temperature at the filament's flange after
    /// the setpoints, in °C, once they've all been measured.
    pub chamber_end_temperature_celsius: Option<f64>,

    /// The operator-entered chamber temperature at the filament's flange
    /// before the setpoints, in °C.
    pub chamber_start_temperature_celsius: f64,

    /// Every scalar input to the fit, recorded when the fit runs.
    pub fit_parameters: Option<ColdResistanceFitParameters>,

    /// One entry per setpoint, in the order measured.
    pub points: Vec<ColdResistancePoint>,

    /// The relay state that gave a positive voltage, once the vertical scale
    /// has been chosen. Never `Nil`.
    pub positive_polarity: Option<Polarity>,

    /// The operator-entered room temperature near the supply in °C, for the
    /// supply's accuracy band.
    pub room_temperature_celsius: f64,

    /// The oscilloscope's vertical scale for every measurement, once it's been
    /// chosen.
    pub vertical_scale_volts_per_division: Option<f64>,
}

/// The measurements at one setpoint, filed by the sign of the voltage rather
/// than by relay state, so nothing downstream needs to know which relay state
/// is which.
#[derive(Debug, Deserialize, Serialize)]
pub struct ColdResistancePoint {
    /// The output of `cold_resistance_point.py`, once it's run.
    pub analysis: Option<ColdResistancePointAnalysis>,

    /// The relay state measured first. With `positive_polarity`, this says
    /// which measurement came first and so which settle followed the ramp from
    /// the previous setpoint. Never `Nil`.
    pub first_polarity: Polarity,

    /// The current in the polarity that gives a negative voltage.
    pub negative_current_amps: Measurement,

    /// How long the filament took to settle in the polarity that gives a
    /// negative voltage, in seconds.
    pub negative_settle_seconds: f64,

    /// The voltage in the polarity that gives a negative voltage.
    pub negative_voltage_volts: Measurement,

    /// The current in the polarity that gives a positive voltage.
    pub positive_current_amps: Measurement,

    /// How long the filament took to settle in the polarity that gives a
    /// positive voltage, in seconds.
    pub positive_settle_seconds: f64,

    /// The voltage in the polarity that gives a positive voltage.
    pub positive_voltage_volts: Measurement,

    /// The current this point was measured at in amperes.
    pub setpoint_amps: f64,

    /// Warnings raised while measuring this point, e.g. a settle timeout.
    pub warnings: Vec<String>,
}

/// Everything measured during one characterisation run.
///
/// Written to disk at checkpoints throughout the procedure so that a run which
/// fails part-way still leaves its partial results behind.
///
/// Every measured field is an `Option`, `None` until it's measured, because the
/// file is saved before anything is — and a run that's quit part-way leaves the
/// rest as `null`.
#[derive(Debug, Deserialize, Serialize)]
pub struct Characterisation {
    /// Everything recorded while measuring the cold resistance, once its
    /// setup has finished.
    pub cold_resistance: Option<ColdResistance>,

    /// An identifier for the filament under test, entered by the operator.
    ///
    /// `None` until then. It's the first thing the procedure asks for, before
    /// any measurement is taken, so a file with a measurement in it always has
    /// one — but a run quit at that first prompt is still saved, and `None`
    /// (`null` in the file) says so honestly where an empty string wouldn't.
    pub filament_id: Option<String>,

    /// Whether the run was in a pumped-down chamber.
    ///
    /// `false` for a bench test with `--no-vacuum`, e.g. against a precision
    /// resistor standing in for a filament, which needs no vacuum. Recorded so
    /// a results file always says which it was.
    pub in_vacuum: bool,

    /// When the run started, in seconds since the Unix epoch.
    pub started_at: u64,
}

impl Characterisation {
    /// Creates the results for a run starting now, `in_vacuum` or not.
    pub fn new(in_vacuum: bool) -> Self {
        Self {
            cold_resistance: None,
            filament_id: None,
            in_vacuum,

            // Only fails if the system clock is set before 1970.
            started_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("the system clock is before the Unix epoch")
                .as_secs(),
        }
    }

    /// Where these results are written.
    ///
    /// Named for when the run started, so successive runs don't overwrite each
    /// other. Derived from `started_at` rather than read from the clock again,
    /// so the name always matches the time recorded inside.
    pub fn path(&self) -> PathBuf {
        PathBuf::from(format!("out/characterisation-{}.json", self.started_at))
    }

    /// Writes the results to `path()`.
    ///
    /// Writes to a temporary file and renames it over the real one, so killing
    /// the program can't leave a half-written file behind — a rename within one
    /// directory is atomic. The directory is created if it doesn't exist.
    pub fn save(&self) -> Result<(), ResultsError> {
        self.save_to(&self.path())
    }

    /// Writes the results to `path`.
    ///
    /// Separate from `save` so the tests can write somewhere other than `out/`:
    /// they run in parallel, all start in the same second, and would otherwise
    /// share one file there.
    ///
    /// The `fs` calls are made inline rather than in `spawn_blocking`: the file
    /// is small, and this matches the vacuum control binary.
    fn save_to(&self, path: &Path) -> Result<(), ResultsError> {
        let json = serde_json::to_string_pretty(self).inspect_err(|e| {
            error!("couldn't serialise the results: {}", e);
        })?;

        let io_error = |path: &Path| {
            let path = path.to_path_buf();
            move |source: std::io::Error| {
                error!(
                    "couldn't write the results to {}: {}",
                    path.display(),
                    source
                );
                ResultsError::Io { path, source }
            }
        };

        if let Some(directory) = path.parent() {
            fs::create_dir_all(directory).map_err(io_error(directory))?;
        }

        let mut temporary = path.as_os_str().to_owned();
        temporary.push("_new");
        let temporary = PathBuf::from(temporary);

        fs::write(&temporary, json).map_err(io_error(&temporary))?;
        fs::rename(&temporary, path).map_err(io_error(path))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory for one test, removed when it's dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(test: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("characteriser-{}-{}", test, std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn load(path: &Path) -> Characterisation {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn save_round_trips() {
        let scratch = Scratch::new("round-trip");
        let path = scratch.0.join("results.json");

        let mut characterisation = Characterisation::new(true);
        characterisation.filament_id = Some(String::from("W-0007"));
        characterisation.save_to(&path).unwrap();

        let loaded = load(&path);
        assert_eq!(loaded.filament_id.as_deref(), Some("W-0007"));
        assert_eq!(loaded.started_at, characterisation.started_at);
    }

    #[test]
    fn save_overwrites_and_leaves_no_temporary_file() {
        let scratch = Scratch::new("overwrite");
        let path = scratch.0.join("results.json");

        let mut characterisation = Characterisation::new(true);
        characterisation.filament_id = Some(String::from("first"));
        characterisation.save_to(&path).unwrap();
        characterisation.filament_id = Some(String::from("second"));
        characterisation.save_to(&path).unwrap();

        assert_eq!(load(&path).filament_id.as_deref(), Some("second"));
        let files: Vec<_> = fs::read_dir(&scratch.0).unwrap().collect();
        assert_eq!(files.len(), 1, "expected only the results file");
    }

    #[test]
    fn save_writes_an_unset_filament_id_as_null() {
        let scratch = Scratch::new("unset-id");
        let path = scratch.0.join("results.json");

        Characterisation::new(true).save_to(&path).unwrap();

        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(json["filament_id"].is_null());
        assert_eq!(load(&path).filament_id, None);
    }

    #[test]
    fn save_creates_the_directory() {
        let scratch = Scratch::new("directory");
        let path = scratch.0.join("out").join("results.json");

        Characterisation::new(true).save_to(&path).unwrap();

        assert!(path.exists());
    }

    #[test]
    fn path_is_named_for_the_start_time() {
        let characterisation = Characterisation::new(true);
        assert_eq!(
            characterisation.path(),
            PathBuf::from(format!(
                "out/characterisation-{}.json",
                characterisation.started_at
            ))
        );
    }
}
