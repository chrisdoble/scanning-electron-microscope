//! The characterisation procedure itself.
//!
//! The top-level sections of the run, in order, each an ordinary async function
//! so the procedure reads as the sequence of things that happen. Every section
//! takes the `Hardware`: that's how one reads an instrument more often than the
//! once-a-second snapshot, as `take_samples` does.

mod cold_resistance;
mod measurement;

use super::{Context, ProcedureError};
use crate::{
    constants::{FILAMENT_OPERATING_PRESSURE_MBAR, TMP_MAXIMUM_BACKING_PRESSURE_MBAR},
    hardware::Hardware,
};
use cold_resistance::measure_cold_resistance;
use host::power_supply::Polarity;

/// Characterises a filament.
///
/// Without vacuum (`--no-vacuum`), the chamber is never pumped down, so the TMP
/// is never started or stopped.
pub async fn characterise(ctx: Context, hardware: Hardware) -> Result<(), ProcedureError> {
    ctx.section("Preparing", |ctx| prepare(ctx, hardware.clone()))
        .await?;
    if ctx.in_vacuum() {
        ctx.section("Pumping down chamber", |ctx| {
            pump_down(ctx, hardware.clone())
        })
        .await?;
    } else {
        ctx.text("Running without vacuum, so the chamber isn't pumped down");
    }
    ctx.section("Measuring cold resistance", |ctx| {
        measure_cold_resistance(ctx, hardware.clone())
    })
    .await?;
    ctx.section("Finishing", |ctx| finish(ctx, hardware.clone()))
        .await?;

    // Asked once the filament is safe, since the operator may not answer
    // straight away. Leaving the TMP running means the next run's pump-down
    // passes straight through.
    if ctx.in_vacuum() {
        if ctx
            .ask("Turn off the TMP and wait for it to spin down?")
            .await?
        {
            ctx.section("Spinning down TMP", |ctx| spin_down(ctx, hardware.clone()))
                .await?;
        } else {
            ctx.text("Left the TMP running");
        }
    }
    ctx.confirm("Press enter to exit").await?;
    Ok(())
}

/// Identifies the filament and has the operator confirm the rig is ready.
///
/// Sets no limits on the supply: each measurement sets the ones it needs.
async fn prepare(ctx: Context, _hardware: Hardware) -> Result<(), ProcedureError> {
    // First, before anything is measured, so no results file with a
    // measurement in it can lack one. The first save is also what tells the
    // operator where the results are going.
    let filament_id: String = ctx.input("Filament ID", None).await?;
    ctx.record(|characterisation| characterisation.filament_id = Some(filament_id));
    ctx.save();

    ctx.confirm("Confirm that the filament is mounted").await?;
    if ctx.in_vacuum() {
        ctx.confirm("Confirm that the chamber is sealed").await?;
    }

    // The scope's probe ratio is reset to 1×, so this is what makes that
    // correct. A probe switched to 10× would read 10× low.
    ctx.confirm(
        "Confirm that a 1× probe is connected to the oscilloscope's channel 1 and that its switch is set to 1×",
    )
    .await?;

    // The cold resistance is relative to the mount's temperature at the first
    // reference, which should be the flange's. The mount takes about 15
    // minutes to cool after the filament's been powered, e.g. by an earlier
    // run.
    ctx.confirm("Confirm that the filament hasn't been powered for at least 15 minutes")
        .await?;

    // The supply's accuracy figures assume a 1-hour warm-up, and the
    // pump-down usually covers it.
    ctx.confirm(
        "Confirm that the oscilloscope has been on for at least 30 minutes and the power supply for at least 60 minutes",
    )
    .await?;
    Ok(())
}

/// Brings the chamber down to the pressure the filament is operated at.
async fn pump_down(ctx: Context, hardware: Hardware) -> Result<(), ProcedureError> {
    // Nothing can check this: a low chamber pressure only proves the roughing
    // pump was running, not that it still is.
    ctx.confirm("Confirm that the roughing pump is running")
        .await?;

    ctx.wait_for(
        "Waiting for the chamber to reach the TMP's operating pressure",
        |s| s.vacuum.pressure.value < TMP_MAXIMUM_BACKING_PRESSURE_MBAR,
    )
    .await?;

    hardware.vacuum.set_tmp_running(true).await?;
    ctx.text("Turned on the TMP");

    ctx.wait_for("Waiting for the TMP to reach speed", |s| {
        s.vacuum.tmp_current_rotation_speed >= s.vacuum.tmp_target_rotation_speed
    })
    .await?;

    ctx.wait_for(
        "Waiting for the chamber to reach the filament's operating pressure",
        |s| s.vacuum.pressure.value < FILAMENT_OPERATING_PRESSURE_MBAR,
    )
    .await?;

    ctx.measurement(
        "Base pressure",
        format!("{:.1e} mbar", ctx.snapshot().vacuum.pressure.value),
    );
    Ok(())
}

/// Stops the TMP and waits for it to come to rest.
async fn spin_down(ctx: Context, hardware: Hardware) -> Result<(), ProcedureError> {
    hardware.vacuum.set_tmp_running(false).await?;
    ctx.text("Turned off the TMP");

    ctx.wait_for("Waiting for the TMP to spin down", |s| {
        s.vacuum.tmp_current_rotation_speed == 0
    })
    .await?;
    Ok(())
}

/// Leaves the filament system unpowered and saves a final time.
async fn finish(ctx: Context, hardware: Hardware) -> Result<(), ProcedureError> {
    let filament = &hardware.filament;

    // The output goes off before the relays are switched, which
    // `set_polarity` insists on.
    filament.set_heating_current_limit(0.0).await?;
    filament.set_output_enabled(false).await?;
    filament.set_polarity(Polarity::Nil).await?;
    ctx.text("Zeroed the current, disabled the output and de-energised the relays");

    ctx.save();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        hardware::{self, MockFilamentSystem, MockVacuumSystem},
        procedure,
        results::{Characterisation, ColdResistancePoint},
        steps::{Section, StepKind},
    };
    use cold_resistance::REFERENCE_CURRENT_AMPS;
    use std::{
        fs,
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tokio::sync::watch;
    use tokio_util::sync::CancellationToken;

    /// Answers whichever prompt is waiting, as the operator would: confirms a
    /// gate, enters a value for an input, or answers a question with
    /// `spin_down`, the only question asked.
    fn answer_prompt(root: &Mutex<Section>, spin_down: bool) {
        let mut root = root.lock().unwrap();
        let Some(step) = root.pending_mut() else {
            return;
        };

        match &mut step.kind {
            StepKind::Confirm { responder, .. } => {
                if let Some(responder) = responder.take() {
                    let _ = responder.send(());
                }
            }
            StepKind::Input {
                prompt, responder, ..
            } => {
                let value = match prompt.as_str() {
                    "Filament ID" => "test",
                    "Room temperature near the power supply" => "21",
                    _ => "22",
                };
                if let Some(responder) = responder.take() {
                    let _ = responder.send(String::from(value));
                }
            }
            StepKind::Question { responder, .. } => {
                if let Some(responder) = responder.take() {
                    let _ = responder.send(spin_down);
                }
            }
            _ => {}
        }
    }

    /// Runs the whole procedure against the mocks, `in_vacuum` or not, answering
    /// its prompts, and whether to spin down the TMP with `spin_down`. Returns
    /// the context, for the results, and the step tree.
    ///
    /// Call from a test on a paused clock, so tokio skips through the ramps and
    /// settles instead of waiting minutes for them. Needs the Python virtual
    /// environment, for the analysis scripts.
    async fn run_against_mocks(in_vacuum: bool, spin_down: bool) -> (Context, Arc<Mutex<Section>>) {
        let hardware = Hardware {
            filament: Arc::new(MockFilamentSystem::default()),
            vacuum: Arc::new(MockVacuumSystem::default()),
        };
        let root = Arc::new(Mutex::new(Section::default()));
        let (snapshots_tx, snapshots_rx) = watch::channel(None);
        let ctx = Context::new(
            CancellationToken::new(),
            Characterisation::new(in_vacuum),
            Arc::clone(&root),
            snapshots_rx,
        );

        let operator = async {
            loop {
                answer_prompt(&root, spin_down);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        };

        let result = tokio::select! {
            result = procedure::run(ctx.clone(), hardware.clone()) => result,
            e = hardware::poll(hardware.clone(), &snapshots_tx) => panic!("the poll failed: {}", e),
            _ = operator => unreachable!("the operator never stops"),
        };

        // Saving writes to `out/`, which a test shouldn't leave behind.
        let _ = fs::remove_file(ctx.characterisation.lock().unwrap().path());
        result.unwrap();

        (ctx, root)
    }

    /// Checks the run measured every setpoint and reference and found the mock
    /// filament's 0.096 Ω, which has no self-heating.
    fn assert_measured_the_mock_filament(ctx: &Context) {
        let characterisation = ctx.characterisation.lock().unwrap();
        let cold_resistance = characterisation.cold_resistance.as_ref().unwrap();
        assert_eq!(cold_resistance.points.len(), 9);
        assert_eq!(cold_resistance.references.len(), 6);
        assert!(
            cold_resistance
                .points
                .iter()
                .chain(&cold_resistance.references)
                .all(|point| point.analysis.is_some())
        );
        assert!(
            cold_resistance
                .references
                .iter()
                .all(|reference| reference.setpoint_amps == REFERENCE_CURRENT_AMPS)
        );

        // Every polarity's time, in the order they were measured.
        let positive_polarity = cold_resistance.positive_polarity.unwrap();
        let times = |points: &[ColdResistancePoint]| -> Vec<f64> {
            points
                .iter()
                .flat_map(|point| {
                    let positive = point.positive_time_seconds;
                    let negative = point.negative_time_seconds;
                    if point.first_polarity == positive_polarity {
                        [positive, negative]
                    } else {
                        [negative, positive]
                    }
                })
                .collect()
        };
        let point_times = times(&cold_resistance.points);
        let reference_times = times(&cold_resistance.references);
        for times in [&point_times, &reference_times] {
            assert!(
                times.windows(2).all(|pair| pair[0] < pair[1]),
                "expected increasing times, got {:?}",
                times
            );
        }

        // Every point is bracketed by references.
        assert!(reference_times.first() < point_times.first());
        assert!(reference_times.last() > point_times.last());

        let analysis = cold_resistance.analysis.as_ref().unwrap();
        let resistance = analysis.resistance_ohms.value;
        assert!(
            (resistance - 0.096).abs() < 1e-9,
            "expected 0.096 Ω, got {} Ω",
            resistance
        );

        // The mock doesn't drift, so nothing is corrected.
        assert_eq!(analysis.reference_drift_ohms.value, 0.0);
        assert_eq!(analysis.corrected_resistances_ohms.len(), 9);
        assert!(
            analysis
                .drift_corrections_ohms
                .iter()
                .all(|drift| drift.value.abs() < 1e-12)
        );
    }

    /// The titles of the top-level sections the run went through.
    fn section_titles(root: &Mutex<Section>) -> Vec<String> {
        root.lock()
            .unwrap()
            .children
            .iter()
            .filter_map(|step| match &step.kind {
                StepKind::Section(section) => Some(section.title.clone()),
                _ => None,
            })
            .collect()
    }

    #[tokio::test(start_paused = true)]
    async fn the_procedure_measures_the_mock_filament() {
        let (ctx, root) = run_against_mocks(true, true).await;
        assert_measured_the_mock_filament(&ctx);
        assert!(ctx.characterisation.lock().unwrap().in_vacuum);

        let titles = section_titles(&root);
        assert!(titles.iter().any(|title| title == "Pumping down chamber"));
        assert!(titles.iter().any(|title| title == "Spinning down TMP"));
    }

    #[tokio::test(start_paused = true)]
    async fn the_procedure_can_leave_the_tmp_running() {
        let (ctx, root) = run_against_mocks(true, false).await;
        assert_measured_the_mock_filament(&ctx);

        let titles = section_titles(&root);
        assert!(!titles.iter().any(|title| title == "Spinning down TMP"));
        assert!(root.lock().unwrap().children.iter().any(|step| matches!(
            &step.kind,
            StepKind::Text { spans } if spans.iter().any(|span| span.content == "Left the TMP running")
        )));
    }

    // Without vacuum, the chamber is never pumped down and the TMP is never
    // started or stopped, but the measurement is the same.
    #[tokio::test(start_paused = true)]
    async fn the_procedure_measures_the_mock_filament_without_vacuum() {
        let (ctx, root) = run_against_mocks(false, true).await;
        assert_measured_the_mock_filament(&ctx);
        assert!(!ctx.characterisation.lock().unwrap().in_vacuum);

        let titles = section_titles(&root);
        assert!(!titles.iter().any(|title| title == "Pumping down chamber"));
        assert!(!titles.iter().any(|title| title == "Spinning down TMP"));
    }
}
