use crate::{
    AnyError,
    steps::{Section, StepKind},
    ui,
    ui::StepsState,
};
use crossterm::event::{Event, EventStream, KeyCode, KeyModifiers};
use futures::StreamExt;
use log::*;
use ratatui::DefaultTerminal;
use std::{
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};
use tokio::time::MissedTickBehavior;

/// How often the UI is redrawn.
///
/// 20 fps, which is enough for the spinner on a running step to animate
/// smoothly.
const RENDER_INTERVAL: Duration = Duration::from_millis(50);

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

    /// Scroll position of the steps list.
    steps_state: StepsState,
}

impl App {
    pub fn new(root: Arc<Mutex<Section>>) -> Self {
        Self {
            root,
            steps_state: StepsState::default(),
        }
    }

    /// Runs the application's event loop until the user quits.
    ///
    /// The terminal is drawn from here rather than from a task of its own
    /// because `DefaultTerminal` would otherwise have to be shared.
    pub async fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<(), AnyError> {
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

        // `tokio::select!` polls every branch's future concurrently and, when
        // one completes, drops the others. Dropping a future cancels whatever
        // work it was doing, so the question isn't whether the other branches
        // are cancelled — they are — but whether cancelling them loses
        // anything. Both of these are cancellation-safe, because their state
        // lives in a long-lived object outside the future: an unread key event
        // stays in the `EventStream`'s buffer, and the tick deadline lives in
        // the `Interval`. Each iteration creates fresh futures from those
        // objects, so a branch that lost the race is simply re-awaited with
        // nothing missed.
        //
        // IMPORTANT: only cancellation-safe futures go directly in a `select!`
        // branch. Anything that buffers into a local (a multi-step read, a
        // partially consumed iterator) must be driven by a task and its result
        // delivered over a channel instead.
        loop {
            tokio::select! {
                Some(Ok(event)) = terminal_events.next() => {
                    if self.handle_terminal_event(event)? {
                        info!("Quitting");
                        break;
                    }
                }
                _ = ticker.tick() => self.render(terminal)?,
            }
        }

        Ok(())
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

    /// Draws the UI.
    fn render(&mut self, terminal: &mut DefaultTerminal) -> Result<(), AnyError> {
        // Borrow the two fields separately — `lock` is a free function rather
        // than a method so that holding the guard doesn't borrow all of `self`.
        let root = lock(&self.root)?;
        let steps_state = &mut self.steps_state;
        terminal.draw(|frame| ui::render(frame, &root, steps_state))?;
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
