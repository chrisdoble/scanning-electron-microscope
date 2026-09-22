use crate::{AnyError, ui};
use crossterm::event::{Event, EventStream, KeyCode, KeyModifiers};
use futures::StreamExt;
use log::*;
use ratatui::DefaultTerminal;
use std::time::{Duration, Instant};
use tokio::time::MissedTickBehavior;

/// How often the UI is redrawn.
///
/// 20 fps, which is enough for the spinner on a running step to animate
/// smoothly.
const RENDER_INTERVAL: Duration = Duration::from_millis(50);

/// The application.
#[derive(Debug)]
pub struct App {
    /// When the application started.
    start_time: Instant,
}

impl App {
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
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
                    if self.handle_terminal_event(event) {
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
    fn handle_terminal_event(&mut self, event: Event) -> bool {
        // Only act on presses, otherwise key repeats double-fire on terminals
        // that report them.
        let Event::Key(event) = event else {
            return false;
        };
        if !event.is_press() {
            return false;
        }

        match event.code {
            // Ctrl+C always quits immediately.
            KeyCode::Char('c') | KeyCode::Char('C')
                if event.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                true
            }

            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => true,

            _ => false,
        }
    }

    /// Draws the UI.
    fn render(&self, terminal: &mut DefaultTerminal) -> Result<(), AnyError> {
        terminal.draw(|frame| ui::render(frame, self.start_time.elapsed()))?;
        Ok(())
    }
}
