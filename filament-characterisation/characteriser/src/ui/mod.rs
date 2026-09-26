mod filament;
mod run;
mod steps;
mod vacuum;

pub use steps::{Steps, StepsState};

use crate::{
    hardware::Snapshots,
    steps::{Section, StepKind},
    style::{BLOCK_TITLE_STYLE, SUBTLE_TEXT_STYLE, VARIABLE_STYLE},
};
use filament::FilamentBlock;
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::Styled,
    text::{Line, Span},
    widgets::{Block, Padding, Paragraph, Widget},
};
use run::RunBlock;
use std::time::Duration;
use vacuum::VacuumBlock;

/// Renders the application.
pub fn render(
    frame: &mut Frame,
    root: &Section,
    steps_state: &mut StepsState,
    snapshots: Option<&Snapshots>,
    elapsed: Duration,
    exiting: bool,
) {
    let [top, steps, shortcuts_bar] = Layout::vertical([
        Constraint::Length(6),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    let [vacuum, filament, run] = Layout::horizontal([Constraint::Ratio(1, 3); 3]).areas(top);

    frame.render_widget(
        VacuumBlock {
            snapshot: snapshots.map(|s| &s.vacuum),
        },
        vacuum,
    );
    frame.render_widget(
        FilamentBlock {
            snapshot: snapshots.map(|s| &s.filament),
        },
        filament,
    );
    frame.render_widget(RunBlock { elapsed, root }, run);
    frame.render_stateful_widget(Steps { root }, steps, steps_state);

    // The shortcuts bar's text changes with context, as in the vacuum control
    // binary. Scrolling isn't offered while a step is pending, because the view
    // stays pinned to it until it's answered.
    let shortcuts = match root.pending().map(|step| &step.kind) {
        _ if exiting => "[Any key] Exit",
        Some(StepKind::Confirm { .. }) => "[Enter] Confirm   [Esc] Quit",
        Some(StepKind::Input { .. }) => "[Enter] Submit   [Esc] Quit",
        _ => "[↑/↓] Scroll   [Esc] Quit",
    };
    frame.render_widget(
        Paragraph::new(shortcuts.set_style(SUBTLE_TEXT_STYLE)).centered(),
        shortcuts_bar,
    );
}

/// A bordered block titled `title`, as the top blocks all are.
///
/// Padded horizontally only: the top row is six high, which is the border plus
/// exactly four rows, so there's no room for vertical padding.
fn block(title: &str) -> Block<'_> {
    Block::bordered()
        .padding(Padding::horizontal(1))
        .title(title.set_style(BLOCK_TITLE_STYLE))
}

/// Labelled values, one per line, fitted to `width` columns.
///
/// The label is in the default style and the value in `VARIABLE_STYLE`, the
/// two-tone idiom the vacuum control binary uses for every reading. Labels are
/// padded to a common width so the values line up, and a value too long to fit
/// ends in `…` rather than being cut off silently.
fn rows(width: u16, rows: &[(&str, String)]) -> Vec<Line<'static>> {
    let label_width = rows
        .iter()
        .map(|(label, _)| label.len() + 1)
        .max()
        .unwrap_or(0);
    let value_width = (width as usize).saturating_sub(label_width + 1);

    rows.iter()
        .map(|(label, value)| {
            let value = if value.chars().count() > value_width {
                let kept: String = value.chars().take(value_width.saturating_sub(1)).collect();
                format!("{}…", kept)
            } else {
                value.clone()
            };

            Line::from(vec![
                Span::raw(format!(
                    "{:width$} ",
                    format!("{}:", label),
                    width = label_width
                )),
                Span::styled(value, VARIABLE_STYLE),
            ])
        })
        .collect()
}

/// Draws the state a block is in before the first poll has been published.
fn waiting(area: Rect, buf: &mut Buffer) {
    Paragraph::new("Waiting for readings…".set_style(SUBTLE_TEXT_STYLE))
        .centered()
        .render(
            area.centered(Constraint::Length(area.width), Constraint::Length(1)),
            buf,
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
