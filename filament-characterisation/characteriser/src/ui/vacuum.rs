use super::{block, rows, waiting};
use crate::hardware::VacuumSnapshot;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    widgets::{Paragraph, Widget},
};

/// The vacuum system's latest readings.
pub struct VacuumBlock<'a> {
    /// `None` until the first poll has been published.
    pub snapshot: Option<&'a VacuumSnapshot>,
}

impl Widget for VacuumBlock<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = block("Vacuum");
        let inner = block.inner(area);
        block.render(area, buf);

        let Some(snapshot) = self.snapshot else {
            return waiting(inner, buf);
        };

        Paragraph::new(rows(
            inner.width,
            &[
                (
                    "Pressure",
                    format!("{:.1e} {}", snapshot.pressure.value, snapshot.pressure.unit),
                ),
                (
                    "TMP",
                    String::from(if snapshot.tmp_running {
                        "Running"
                    } else {
                        "Stopped"
                    }),
                ),
                (
                    "Speed",
                    format!(
                        "{} / {} Hz",
                        snapshot.tmp_current_rotation_speed, snapshot.tmp_target_rotation_speed
                    ),
                ),
                ("Current", format!("{:.2} A", snapshot.tmp_current)),
            ],
        ))
        .render(inner, buf);
    }
}
