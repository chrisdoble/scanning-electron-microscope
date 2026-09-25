//! What a characterisation run measures, and how it's written to disk.

use log::*;
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
#[derive(Debug, Deserialize, Serialize)]
pub struct Measurement {
    /// The individual samples, in the order they were taken.
    pub samples: Vec<f64>,

    /// The standard error of the mean.
    pub uncertainty: f64,

    /// The mean of the samples.
    pub value: f64,
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
    /// The current through the filament while measuring its cold resistance
    /// in `Polarity::Forward`.
    pub cold_forward_current_amps: Option<Measurement>,

    /// The voltage across the filament while measuring its cold resistance in
    /// `Polarity::Forward`.
    pub cold_forward_voltage_volts: Option<Measurement>,

    /// The current through the filament while measuring its cold resistance
    /// in `Polarity::Reverse`.
    pub cold_reverse_current_amps: Option<Measurement>,

    /// The voltage across the filament while measuring its cold resistance in
    /// `Polarity::Reverse`.
    pub cold_reverse_voltage_volts: Option<Measurement>,

    /// An identifier for the filament under test, entered by the operator.
    ///
    /// `None` until then. It's the first thing the procedure asks for, before
    /// any measurement is taken, so a file with a measurement in it always has
    /// one — but a run quit at that first prompt is still saved, and `None`
    /// (`null` in the file) says so honestly where an empty string wouldn't.
    pub filament_id: Option<String>,

    /// When the run started, in seconds since the Unix epoch.
    pub started_at: u64,
}

impl Characterisation {
    /// Creates the results for a run starting now.
    pub fn new() -> Self {
        Self {
            cold_forward_current_amps: None,
            cold_forward_voltage_volts: None,
            cold_reverse_current_amps: None,
            cold_reverse_voltage_volts: None,
            filament_id: None,

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

        let mut characterisation = Characterisation::new();
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

        let mut characterisation = Characterisation::new();
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

        Characterisation::new().save_to(&path).unwrap();

        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(json["filament_id"].is_null());
        assert_eq!(load(&path).filament_id, None);
    }

    #[test]
    fn save_creates_the_directory() {
        let scratch = Scratch::new("directory");
        let path = scratch.0.join("out").join("results.json");

        Characterisation::new().save_to(&path).unwrap();

        assert!(path.exists());
    }

    #[test]
    fn path_is_named_for_the_start_time() {
        let characterisation = Characterisation::new();
        assert_eq!(
            characterisation.path(),
            PathBuf::from(format!(
                "out/characterisation-{}.json",
                characterisation.started_at
            ))
        );
    }
}
