mod steps;

pub use steps::{Steps, StepsState};

use crate::{
    steps::{Section, StepKind},
    style::SUBTLE_TEXT_STYLE,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::Styled,
    widgets::Paragraph,
};
use std::time::Duration;

/// Renders the application.
///
/// TODO: add the vacuum, filament and run blocks above the steps list, as
/// described in section 14 of the design document.
pub fn render(frame: &mut Frame, root: &Section, steps_state: &mut StepsState) {
    let [steps, shortcuts_bar] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());

    frame.render_stateful_widget(Steps { root }, steps, steps_state);

    // The shortcuts bar's text changes with context, as in the vacuum control
    // binary. Scrolling isn't offered while a step is pending, because the view
    // stays pinned to it until it's answered.
    let shortcuts = match root.pending().map(|step| &step.kind) {
        Some(StepKind::Confirm { .. }) => "[Enter] Confirm   [Esc] Quit",
        Some(StepKind::Input { .. }) => "[Enter] Submit   [Esc] Quit",
        _ => "[↑/↓] Scroll   [Esc] Quit",
    };
    frame.render_widget(
        Paragraph::new(shortcuts.set_style(SUBTLE_TEXT_STYLE)).centered(),
        shortcuts_bar,
    );
}

/// Formats a duration as `3s`, `2m 3s` or `1h 2m 3s`, leaving out the units
/// that would be zero.
pub fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);

    if hours > 0 {
        format!("{}h {}m {}s", hours, minutes, seconds)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, seconds)
    } else {
        format!("{}s", seconds)
    }
}
