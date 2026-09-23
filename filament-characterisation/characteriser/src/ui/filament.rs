use super::{block, rows, waiting};
use crate::hardware::FilamentSnapshot;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    widgets::{Paragraph, Widget},
};

/// The filament system's latest readings.
pub struct FilamentBlock<'a> {
    /// `None` until the first poll has been published.
    pub snapshot: Option<&'a FilamentSnapshot>,
}

impl Widget for FilamentBlock<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = block("Filament");
        let inner = block.inner(area);
        block.render(area, buf);

        let Some(snapshot) = self.snapshot else {
            return waiting(inner, buf);
        };

        Paragraph::new(rows(
            inner.width,
            &[
                ("Current", format!("{:.2} A", snapshot.heating_current)),
                ("Polarity", snapshot.polarity.to_string()),
                (
                    "Output",
                    String::from(if snapshot.output_enabled {
                        "Enabled"
                    } else {
                        "Disabled"
                    }),
                ),
                (
                    "Voltage",
                    format!("{:.1} mV", snapshot.filament_voltage * 1000.0),
                ),
            ],
        ))
        .render(inner, buf);
    }
}
