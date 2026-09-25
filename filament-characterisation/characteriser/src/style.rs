//! The styles shared by the step tree and the UI.
//!
//! All but `SUCCESS_STYLE` are taken from the vacuum control binary so the two
//! applications look alike.

use ratatui::style::{Color, Modifier, Style};

pub const BLOCK_TITLE_STYLE: Style = Style::new().add_modifier(Modifier::BOLD).fg(Color::White);
pub const CONFIRMATION_STYLE: Style = Style::new().fg(Color::Yellow);
pub const ERROR_STYLE: Style = Style::new().fg(Color::Red);
pub const SUBTLE_TEXT_STYLE: Style = Style::new().fg(Color::Rgb(96, 96, 96));
pub const SUCCESS_STYLE: Style = Style::new().fg(Color::Green);
pub const VARIABLE_STYLE: Style = Style::new().fg(Color::Blue);
