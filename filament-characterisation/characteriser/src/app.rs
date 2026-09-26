use crate::{
    AnyError,
    hardware::{HardwareError, Snapshots},
    procedure::ProcedureError,
    steps::{Section, StepKind, StepStatus},
    ui,
    ui::StepsState,
};
use crossterm::event::{Event, EventStream, KeyCode, KeyModifiers};
use futures::StreamExt;
use log::*;
use ratatui::DefaultTerminal;
use std::{
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};
use tokio::{
    signal::unix::{SignalKind, signal},
    sync::{mpsc, watch},
    time::MissedTickBehavior,
};

/// How often the UI is redrawn.
///
/// 20 fps, which is enough for the spinner on a running step to animate
/// smoothly.
const RENDER_INTERVAL: Duration = Duration::from_millis(50);

/// An out-of-band message to the application's event loop.
///
/// Only what can't be expressed as shared state: the step tree is shared, and
/// the snapshots have a watch channel of their own.
#[derive(Debug)]
pub enum AppEvent {
    /// The hardware has failed persistently and the run can't continue.
    HardwareFailed(HardwareError),

    /// The procedure task has finished, successfully or otherwise.
    ProcedureFinished(Result<(), ProcedureError>),

    /// The procedure task panicked, with this message.
    ProcedurePanicked(String),
}

/// Why the application's event loop stopped.
///
/// Whatever the reason, `main` then cancels the procedure, waits for it, and
/// puts the filament system into a safe state — in that order, and before
/// anything waits on the operator.
#[derive(Debug)]
pub enum Shutdown {
    /// The procedure finished, the operator quit, or a signal asked us to stop.
    Normal,

    /// The run failed. The message is already shown as a failed step; this copy
    /// is what `main` reports once the terminal is restored.
    Error(String),

    /// The procedure task panicked. `ratatui::init`'s panic hook has already
    /// restored the terminal, so there's nothing left to show the error on.
    Panicked(String),
}

/// The application.
#[derive(Debug)]
pub struct App {
    /// The step tree.
    ///
    /// Shared with the procedure task, which mutates it while the application
    /// renders it. The lock is a `std::sync::Mutex` because every critical
    /// section is either a short synchronous mutation or one render pass, and
    /// it is never held across an `.await`. The application takes it once per
    /// frame, for the duration of the draw.
    root: Arc<Mutex<Section>>,

    /// The most recent readings, or `None` before the first poll.
    ///
    /// Once the first arrives it stays `Some`: a failed poll never clears it.
    snapshots: Option<Snapshots>,

    /// When the application started.
    start_time: Instant,

    /// Scroll position of the steps list.
    steps_state: StepsState,
}

impl App {
    pub fn new(root: Arc<Mutex<Section>>) -> Self {
        Self {
            root,
            snapshots: None,
            start_time: Instant::now(),
            steps_state: StepsState::default(),
        }
    }

    /// Shows the final state of a failed run and waits for any key.
    ///
    /// Called once the filament system is already safe, so the operator can
    /// read what went wrong before the terminal is restored. A signal ends the
    /// wait too, so `kill` still works while it's up.
    pub async fn acknowledge(&mut self, terminal: &mut DefaultTerminal) -> Result<(), AnyError> {
        // `run`'s `EventStream` was dropped when it returned, so this is the
        // only one. The branches are cancellation-safe for the same reasons as
        // `run`'s.
        let mut terminal_events = EventStream::new();
        let mut hangup = signal(SignalKind::hangup())?;
        let mut terminate = signal(SignalKind::terminate())?;
        let mut ticker = tokio::time::interval(RENDER_INTERVAL);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                Some(Ok(Event::Key(event))) = terminal_events.next() => {
                    if event.is_press() {
                        return Ok(());
                    }
                }
                _ = hangup.recv() => return Ok(()),
                _ = terminate.recv() => return Ok(()),
                _ = ticker.tick() => self.render(terminal, true)?,
            }
        }
    }

    /// Runs the application's event loop until the run ends, returning why.
    ///
    /// The terminal is drawn from here rather than from a task of its own
    /// because `DefaultTerminal` would otherwise have to be shared.
    ///
    /// `snapshots` carries the hardware poll task's readings, and `events` the
    /// messages that can't be expressed as shared state.
    pub async fn run(
        &mut self,
        terminal: &mut DefaultTerminal,
        mut snapshots: watch::Receiver<Option<Snapshots>>,
        mut events: mpsc::Receiver<AppEvent>,
    ) -> Result<Shutdown, AnyError> {
        // `EventStream` needs no wiring to the terminal: it reads crossterm's
        // process-wide event source, which `ratatui::init` has already put into
        // raw mode. Only one may exist at a time, and nothing else may read
        // events while it does, or they'll contend for that source.
        let mut terminal_events = EventStream::new();

        let mut ticker = tokio::time::interval(RENDER_INTERVAL);

        // A tokio interval bursts by default: after a stall it fires every
        // missed tick back to back to catch up. That's wrong for rendering,
        // where one draw brings the screen fully up to date and the rest would
        // be identical frames. `Delay` instead guarantees a whole
        // `RENDER_INTERVAL` between draws, however late one runs.
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

        // Closing the terminal sends SIGHUP and `kill` sends SIGTERM, and by
        // default either ends the process on the spot, with no cleanup. Handled
        // here, they shut down like a quit. (Ctrl+C needs nothing: raw mode
        // delivers it as a key.)
        let mut hangup = signal(SignalKind::hangup())?;
        let mut terminate = signal(SignalKind::terminate())?;

        // `tokio::select!` polls every branch's future concurrently and, when
        // one completes, drops the others. Dropping a future cancels whatever
        // work it was doing, so the question isn't whether the other branches
        // are cancelled — they are — but whether cancelling them loses
        // anything. All of these are cancellation-safe, because their state
        // lives in a long-lived object outside the future: an unreceived
        // message stays in the channel, an unread key event stays in the
        // `EventStream`'s buffer, the watch receiver's "seen" marker only
        // advances when `changed()` actually resolves, a signal that arrives
        // between polls is held by its `Signal`, and the tick deadline lives in
        // the `Interval`. Each iteration creates fresh futures from
        // those objects, so a branch that lost the race is simply re-awaited
        // with nothing missed.
        //
        // Once the poll task has exited, `changed()` returns an error at once.
        // That only disables its branch for the iteration — the loop still
        // waits on the others, so it doesn't spin.
        //
        // IMPORTANT: only cancellation-safe futures go directly in a `select!`
        // branch. Anything that buffers into a local (a multi-step read, a
        // partially consumed iterator) must be driven by a task and its result
        // delivered over a channel instead.
        loop {
            tokio::select! {
                Some(event) = events.recv() => return self.handle_app_event(event),
                Ok(()) = snapshots.changed() => {
                    self.snapshots = *snapshots.borrow_and_update();
                }
                Some(Ok(event)) = terminal_events.next() => {
                    if self.handle_terminal_event(event)? {
                        info!("Quitting");
                        return Ok(Shutdown::Normal);
                    }
                }
                _ = hangup.recv() => {
                    info!("Received SIGHUP, shutting down");
                    return Ok(Shutdown::Normal);
                }
                _ = terminate.recv() => {
                    info!("Received SIGTERM, shutting down");
                    return Ok(Shutdown::Normal);
                }
                _ = ticker.tick() => self.render(terminal, false)?,
            }
        }
    }

    /// Adds a step saying whether the filament system was made safe, for the
    /// acknowledgement screen.
    pub fn show_cleanup(&self, cleanup: &Result<(), String>) -> Result<(), AnyError> {
        match cleanup {
            Ok(()) => self.push(
                StepKind::text("Put the filament system into a safe state"),
                StepStatus::Done,
            ),
            Err(e) => self.push(
                StepKind::error(format!("The filament system may still be powered: {}", e)),
                StepStatus::Failed,
            ),
        }
    }

    /// Handles a message from another task. Every one of them ends the run.
    ///
    /// A failure is added at the end of the step list, where the operator is
    /// looking. The sections it happened in are marked failed as the procedure
    /// unwinds.
    fn handle_app_event(&mut self, event: AppEvent) -> Result<Shutdown, AnyError> {
        match event {
            AppEvent::HardwareFailed(e) => {
                error!("the hardware has failed: {}", e);
                let message = format!("The hardware failed: {}", e);
                self.push(StepKind::error(message.clone()), StepStatus::Failed)?;
                Ok(Shutdown::Error(message))
            }

            // `procedure::run` has already logged how it ended.
            AppEvent::ProcedureFinished(Ok(())) => Ok(Shutdown::Normal),
            AppEvent::ProcedureFinished(Err(e)) => {
                let message = format!("The procedure failed: {}", e);
                self.push(StepKind::error(message.clone()), StepStatus::Failed)?;
                Ok(Shutdown::Error(message))
            }

            AppEvent::ProcedurePanicked(message) => {
                error!("the procedure panicked: {}", message);
                Ok(Shutdown::Panicked(format!(
                    "The procedure panicked: {}",
                    message
                )))
            }
        }
    }

    /// Handles a terminal event, returning whether the application should quit.
    fn handle_terminal_event(&mut self, event: Event) -> Result<bool, AnyError> {
        // Only act on presses, otherwise key repeats double-fire on terminals
        // that report them.
        let Event::Key(event) = event else {
            return Ok(false);
        };
        if !event.is_press() {
            return Ok(false);
        }

        let control = event.modifiers.contains(KeyModifiers::CONTROL);
        let mut root = lock(&self.root)?;

        // Taking a responder out of the step is what stops it being pending, so
        // a second `Enter` can't answer twice. Sending on it is what wakes the
        // procedure, which then sets the step's status: the application only
        // sends. An error sending only means the procedure has gone away.
        match (root.pending_mut().map(|step| &mut step.kind), event.code) {
            // Quit, whatever's pending.
            (_, KeyCode::Char('c') | KeyCode::Char('C')) if control => return Ok(true),
            (_, KeyCode::Esc) => return Ok(true),

            (Some(StepKind::Confirm { responder, .. }), KeyCode::Enter) => {
                if let Some(responder) = responder.take() {
                    let _ = responder.send(());
                }
            }

            // A pending input takes printable characters, so a `q` typed into
            // it is a character rather than a quit.
            (Some(StepKind::Input { buffer, .. }), KeyCode::Char(c))
                if !control && !event.modifiers.contains(KeyModifiers::ALT) =>
            {
                buffer.push(c);
            }
            (Some(StepKind::Input { buffer, .. }), KeyCode::Backspace) => {
                buffer.pop();
            }
            (
                Some(StepKind::Input {
                    buffer, responder, ..
                }),
                KeyCode::Enter,
            ) => {
                if let Some(responder) = responder.take() {
                    let _ = responder.send(buffer.clone());
                }
            }

            // With nothing pending, `q` quits and the view scrolls. While
            // something is pending the view is pinned to it anyway.
            (None, KeyCode::Char('q') | KeyCode::Char('Q')) => return Ok(true),
            (None, KeyCode::Up) => self.steps_state.scroll_up(1),
            (None, KeyCode::Down) => self.steps_state.scroll_down(1),
            (None, KeyCode::PageUp) => self.steps_state.scroll_up(self.steps_state.page()),
            (None, KeyCode::PageDown) => self.steps_state.scroll_down(self.steps_state.page()),
            (None, KeyCode::Home) => self.steps_state.scroll_to_top(),
            (None, KeyCode::End) => self.steps_state.scroll_to_bottom(),

            _ => {}
        }

        Ok(false)
    }

    /// Appends a finished step to the root of the tree.
    fn push(&self, kind: StepKind, status: StepStatus) -> Result<(), AnyError> {
        let mut root = lock(&self.root)?;
        let index = root.push(kind);
        let step = &mut root.children[index];
        step.finished_at = Some(Instant::now());
        step.status = status;
        Ok(())
    }

    /// Draws the UI. `exiting` is set while waiting for the operator to
    /// acknowledge a failed run.
    fn render(&mut self, terminal: &mut DefaultTerminal, exiting: bool) -> Result<(), AnyError> {
        // Borrow the two fields separately — `lock` is a free function rather
        // than a method so that holding the guard doesn't borrow all of `self`.
        let root = lock(&self.root)?;
        let steps_state = &mut self.steps_state;
        let snapshots = self.snapshots.as_ref();
        let elapsed = self.start_time.elapsed();
        terminal
            .draw(|frame| ui::render(frame, &root, steps_state, snapshots, elapsed, exiting))?;
        Ok(())
    }
}

/// Locks the step tree.
///
/// A poisoned lock is fatal: the error ends the event loop, the terminal is
/// restored, and `main` reports it.
fn lock(root: &Mutex<Section>) -> Result<MutexGuard<'_, Section>, AnyError> {
    root.lock().map_err(|e| {
        error!("failed to acquire the step tree's mutex: {}", e);
        format!("failed to acquire the step tree's mutex: {}", e).into()
    })
}
