use ratatui::{
    Frame,
    layout::Constraint,
    style::{Color, Modifier, Style, Styled},
    widgets::{Block, Padding, Paragraph},
};
use std::time::Duration;

const BLOCK_TITLE_STYLE: Style = Style::new().add_modifier(Modifier::BOLD).fg(Color::White);
const SUBTLE_TEXT_STYLE: Style = Style::new().fg(Color::Rgb(96, 96, 96));

/// The size of the placeholder block, including its border and padding.
const PLACEHOLDER_HEIGHT: u16 = 7;
const PLACEHOLDER_WIDTH: u16 = 36;

/// Renders the application.
///
/// TODO: replace this placeholder with the layout described in section 14 of
/// the design document as the vacuum, filament, run and steps blocks are
/// built. The elapsed time is here so that the render tick is visibly running.
pub fn render(frame: &mut Frame, elapsed: Duration) {
    let area = frame.area().centered(
        Constraint::Length(PLACEHOLDER_WIDTH),
        Constraint::Length(PLACEHOLDER_HEIGHT),
    );

    frame.render_widget(
        Paragraph::new(format!(
            "Elapsed: {}\n\nPress [Esc / Q] to quit.",
            format_duration(elapsed)
        ))
        .block(
            Block::bordered()
                .padding(Padding::symmetric(2, 1))
                .title("Characteriser".set_style(BLOCK_TITLE_STYLE)),
        )
        .centered()
        .set_style(SUBTLE_TEXT_STYLE),
        area,
    );
}

/// Formats a duration as `HH:MM:SS`, or as seconds to one decimal place if it's
/// less than a minute.
pub fn format_duration(duration: Duration) -> String {
    if duration.as_secs() < 60 {
        return format!("{:.1} s", duration.as_secs_f64());
    }

    let seconds = duration.as_secs();
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}
