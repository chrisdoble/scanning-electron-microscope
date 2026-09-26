//! The procedure, and the `Context` it uses to drive the UI and record results.

mod characterisation;

use crate::{
    hardware::{Hardware, HardwareError, Snapshots},
    python::PythonError,
    results::Characterisation,
    steps::{Section, Step, StepKind, StepPath, StepStatus},
};
use log::*;
use std::{
    fmt::Display,
    str::FromStr,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use thiserror::Error;
use tokio::sync::{oneshot, watch};
use tokio_util::sync::CancellationToken;

/// Why the procedure stopped before finishing.
#[derive(Debug, Error)]
pub enum ProcedureError {
    /// The run was cancelled while the procedure was waiting on something.
    #[error("cancelled")]
    Cancelled,

    #[error("hardware error: {0}")]
    Hardware(#[from] HardwareError),

    #[error("python error: {0}")]
    Python(#[from] PythonError),
}

/// A handle used by the procedure to drive the UI and record results.
///
/// Cheap to clone; a clone with a different `parent` is what nesting is built
/// from.
#[derive(Clone, Debug)]
pub struct Context {
    /// Cancelled by the application at shutdown, which unblocks anything this
    /// context is waiting on.
    cancel: CancellationToken,

    /// The run's measurements, shared because `Context` is cloned per section.
    characterisation: Arc<Mutex<Characterisation>>,

    /// The section this context's steps are appended to.
    parent: StepPath,

    /// The step tree.
    root: Arc<Mutex<Section>>,

    /// Whether the results have been saved yet, so the first save can say where
    /// they're going. Shared for the same reason as `characterisation`.
    saved: Arc<AtomicBool>,

    /// The readings published by the hardware poll task.
    snapshots: watch::Receiver<Option<Snapshots>>,
}

impl Context {
    /// A context whose steps are appended to the root of `root`.
    pub fn new(
        cancel: CancellationToken,
        characterisation: Characterisation,
        root: Arc<Mutex<Section>>,
        snapshots: watch::Receiver<Option<Snapshots>>,
    ) -> Self {
        Self {
            cancel,
            characterisation: Arc::new(Mutex::new(characterisation)),
            parent: StepPath::new(),
            root,
            saved: Arc::new(AtomicBool::new(false)),
            snapshots,
        }
    }

    /// Waits for the user to confirm that something is the case.
    ///
    /// A gate rather than a question: it returns once they have, and the only
    /// way past it otherwise is quitting.
    pub async fn confirm(&self, prompt: impl Into<String>) -> Result<(), ProcedureError> {
        let (responder, confirmation) = oneshot::channel();
        let path = self.push(StepKind::Confirm {
            prompt: prompt.into(),
            responder: Some(responder),
        });

        // A dropped responder means nobody can confirm any more, which only
        // happens as the application goes away.
        let result = self
            .unless_cancelled(confirmation)
            .await
            .and_then(|confirmation| confirmation.map_err(|_| ProcedureError::Cancelled));

        // Nobody is waiting on it any more, so it mustn't still look like it's
        // waiting on the user: while it held a responder it would stay pending,
        // pinning the view to a prompt that can no longer be answered.
        if result.is_err() {
            self.update(&path, |kind| {
                if let StepKind::Confirm { responder, .. } = kind {
                    *responder = None;
                }
            });
        }

        self.finish(&path, status(&result));
        result
    }

    /// Asks the user for a value and waits until they enter one that parses.
    ///
    /// A value that doesn't parse gets the parse error beside the prompt, and
    /// the user is asked again in place — so the step stays the last in the
    /// tree, which is what keeps `Section::pending` correct.
    pub async fn input<T>(
        &self,
        prompt: impl Into<String>,
        unit: Option<&str>,
    ) -> Result<T, ProcedureError>
    where
        T: FromStr,
        T::Err: Display,
    {
        let (responder, mut submission) = oneshot::channel();
        let path = self.push(StepKind::Input {
            buffer: String::new(),
            error: None,
            prompt: prompt.into(),
            responder: Some(responder),
            unit: unit.map(String::from),
            value: None,
        });

        loop {
            let buffer = match self
                .unless_cancelled(submission)
                .await
                .and_then(|buffer| buffer.map_err(|_| ProcedureError::Cancelled))
            {
                Ok(buffer) => buffer,

                // As in `confirm`, give up the responder so the step stops
                // looking like it's waiting on the user.
                Err(e) => {
                    self.update(&path, |kind| {
                        if let StepKind::Input { responder, .. } = kind {
                            *responder = None;
                        }
                    });
                    self.finish(&path, StepStatus::Failed);
                    return Err(e);
                }
            };

            let buffer = buffer.trim().to_string();
            match buffer.parse::<T>() {
                Ok(parsed) => {
                    self.update(&path, |kind| {
                        if let StepKind::Input { error, value, .. } = kind {
                            *error = None;
                            *value = Some(buffer);
                        }
                    });
                    self.finish(&path, StepStatus::Done);
                    return Ok(parsed);
                }

                Err(e) => {
                    let (responder, next) = oneshot::channel();
                    submission = next;
                    self.update(&path, |kind| {
                        if let StepKind::Input {
                            error,
                            responder: r,
                            ..
                        } = kind
                        {
                            *error = Some(e.to_string());
                            *r = Some(responder);
                        }
                    });
                }
            }
        }
    }

    /// Adds a labelled measurement to the step list.
    pub fn measurement(&self, label: impl Into<String>, value: impl Display) {
        let path = self.push(StepKind::measurement(label, value));
        self.finish(&path, StepStatus::Done);
    }

    /// Records measurements into the run's results.
    ///
    /// This only changes memory. Call `save` to write them to disk.
    ///
    /// `f` runs with the results locked, so it should only touch the
    /// `Characterisation` it's given. Calling `save` or `record` from inside it
    /// would try to take the same lock again on the same thread, which
    /// deadlocks.
    pub fn record(&self, f: impl FnOnce(&mut Characterisation)) {
        f(&mut lock(&self.characterisation));
    }

    /// Writes the run's results to disk.
    ///
    /// The first save adds a step saying where the results are going; later
    /// ones are only logged. A failure is shown as a failed step and logged,
    /// but never aborts the procedure: losing the record is worse than losing
    /// the rest of the run.
    pub fn save(&self) {
        let (path, result) = {
            let characterisation = lock(&self.characterisation);
            (characterisation.path(), characterisation.save())
        };

        match result {
            Ok(()) => {
                info!("Saved results to {}", path.display());
                if !self.saved.swap(true, Ordering::Relaxed) {
                    self.text(format!("Saved results to {}", path.display()));
                }
            }

            // `save` has already logged it.
            Err(e) => {
                let step = self.push(StepKind::error(e.to_string()));
                self.finish(&step, StepStatus::Failed);
            }
        }
    }

    /// Runs `f` inside a new section, and marks the section done when it ends.
    ///
    /// The context passed to `f` is parented to the new section, so any steps
    /// it emits are nested beneath it. On error the section is marked failed
    /// and the error propagates. Whatever `f` returns is passed back, so a
    /// section can hand its measurements to the caller that records them.
    pub async fn section<F, Fut, T>(
        &self,
        title: impl Into<String>,
        f: F,
    ) -> Result<T, ProcedureError>
    where
        F: FnOnce(Context) -> Fut,
        Fut: Future<Output = Result<T, ProcedureError>>,
    {
        let title = title.into();
        info!("Entered section {}", title);

        let path = self.push(StepKind::Section(Section {
            children: Vec::new(),
            title,
        }));

        let result = f(Context {
            parent: path.clone(),
            ..self.clone()
        })
        .await;

        self.finish(&path, status(&result));
        result
    }

    /// The most recent readings.
    pub fn snapshot(&self) -> Snapshots {
        // `run` doesn't start the procedure until the first snapshot has
        // arrived, and once one has the channel never goes back to `None`.
        self.snapshots
            .borrow()
            .expect("the procedure started before the first snapshot")
    }

    /// Adds a line of text.
    pub fn text(&self, text: impl Into<String>) {
        let path = self.push(StepKind::text(text));
        self.finish(&path, StepStatus::Done);
    }

    /// Waits until `predicate` holds for a snapshot, showing a spinner.
    ///
    /// This is how the procedure waits on the chamber, using the readings the
    /// poll task is already taking rather than issuing its own. For anything
    /// that needs reading more often than the poll does, read the hardware
    /// directly inside `waiting` instead.
    pub async fn wait_for(
        &self,
        label: impl Into<String>,
        predicate: impl Fn(&Snapshots) -> bool,
    ) -> Result<(), ProcedureError> {
        let path = self.push(StepKind::text(label));

        // Checks the current snapshot first, then each new one. An error means
        // the poll task has stopped, which it only does when the hardware has
        // failed and the application is shutting down.
        let mut snapshots = self.snapshots.clone();
        let result = self
            .unless_cancelled(async {
                snapshots
                    .wait_for(|snapshot| snapshot.as_ref().is_some_and(&predicate))
                    .await
                    .map(|_| ())
                    .map_err(|_| ProcedureError::Cancelled)
            })
            .await
            .and_then(|result| result);

        self.finish(&path, status(&result));
        result
    }

    /// Runs `f`, showing a spinner labelled `label` until it completes.
    pub async fn waiting<F, Fut, T>(
        &self,
        label: impl Into<String>,
        f: F,
    ) -> Result<T, ProcedureError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, ProcedureError>>,
    {
        let path = self.push(StepKind::text(label));
        let result = self.unless_cancelled(f()).await.and_then(|result| result);
        self.finish(&path, status(&result));
        result
    }

    // Private helpers. `push` and `with_step` are the only places that take the
    // step tree's lock, and neither holds it across an `.await`.

    /// Sets the step's status and `finished_at`.
    fn finish(&self, path: &[usize], status: StepStatus) {
        self.with_step(path, |step| {
            step.finished_at = Some(Instant::now());
            step.status = status;
        });
    }

    /// Appends a step beneath this context's section and returns its path.
    fn push(&self, kind: StepKind) -> StepPath {
        let mut root = lock(&self.root);

        // Sections are only ever appended to, so the path to one stays valid
        // for the whole run.
        let index = root
            .section_at_mut(&self.parent)
            .expect("a context's section exists")
            .push(kind);

        [self.parent.as_slice(), &[index]].concat()
    }

    /// Awaits `future` unless the run is cancelled first.
    async fn unless_cancelled<T>(
        &self,
        future: impl Future<Output = T>,
    ) -> Result<T, ProcedureError> {
        tokio::select! {
            output = future => Ok(output),
            _ = self.cancel.cancelled() => Err(ProcedureError::Cancelled),
        }
    }

    /// Applies `f` to the kind of the step at `path`.
    ///
    /// Used to write a parse error back into an input step before
    /// re-prompting, or the value it accepted once one parses.
    fn update(&self, path: &[usize], f: impl FnOnce(&mut StepKind)) {
        self.with_step(path, |step| f(&mut step.kind));
    }

    /// Applies `f` to the step at `path`.
    fn with_step(&self, path: &[usize], f: impl FnOnce(&mut Step)) {
        // Steps are never removed, so a path that was returned by `push` stays
        // valid for the whole run.
        f(lock(&self.root)
            .step_at_mut(path)
            .expect("a pushed step exists"));
    }
}

/// Runs the procedure, saving its results however it ends.
///
/// Waits for the first snapshot before doing anything, so nothing runs against
/// instruments that haven't answered yet. Returns rather than reporting the
/// result itself, so this module doesn't depend on the application's event
/// type.
pub async fn run(ctx: Context, hardware: Hardware) -> Result<(), ProcedureError> {
    let mut snapshots = ctx.snapshots.clone();
    ctx.unless_cancelled(async {
        snapshots
            .wait_for(Option::is_some)
            .await
            .map(|_| ())
            .map_err(|_| ProcedureError::Cancelled)
    })
    .await
    .and_then(|result| result)?;

    info!("Starting the procedure");
    let result = characterisation::characterise(ctx.clone(), hardware).await;

    // Save once more on both paths, so the final state always reaches disk.
    ctx.save();

    // Cancellation is how every shutdown stops the procedure, including an
    // ordinary quit, so it isn't logged as a failure.
    match &result {
        Ok(()) => info!("the procedure finished"),
        Err(ProcedureError::Cancelled) => info!("the procedure was cancelled"),
        Err(e) => error!("the procedure failed: {}", e),
    }
    result
}

/// Locks `mutex`, recovering the value if a panic poisoned it.
///
/// The procedure must never panic — the panic hook can't put the filament
/// system into a safe state — so a poisoned lock is recovered rather than
/// unwrapped. The critical sections that could have been interrupted are single
/// pushes and assignments, which can't leave the value half-changed.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The status a step finishes with, given how what it was waiting on ended.
fn status<T>(result: &Result<T, ProcedureError>) -> StepStatus {
    match result {
        Ok(_) => StepStatus::Done,
        Err(_) => StepStatus::Failed,
    }
}
