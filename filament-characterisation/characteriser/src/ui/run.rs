use super::{block, format_duration, rows};
use crate::steps::{Section, StepKind, StepStatus};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    widgets::{Paragraph, Widget},
};
use std::time::Duration;

/// How far through the run the procedure is.
pub struct RunBlock<'a> {
    /// How long the application has been running.
    pub elapsed: Duration,

    /// The step tree, whose last top-level section is the one in progress.
    pub root: &'a Section,
}

impl Widget for RunBlock<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = block("Run");
        let inner = block.inner(area);
        block.render(area, buf);

        // There's no list of stages to count against, so the current section is
        // simply the last top-level one.
        let section = self
            .root
            .children
            .iter()
            .rev()
            .find_map(|step| match &step.kind {
                StepKind::Section(section) => Some((section, step.status)),
                _ => None,
            });

        let (title, status) = match section {
            Some((section, status)) => (
                section.title.clone(),
                String::from(match status {
                    StepStatus::Done => "Done",
                    StepStatus::Failed => "Failed",
                    StepStatus::Running => "Running",
                }),
            ),
            None => (String::from("–"), String::from("Starting")),
        };

        Paragraph::new(rows(
            inner.width,
            &[
                ("Elapsed", format_duration(self.elapsed)),
                ("Section", title),
                ("Status", status),
            ],
        ))
        .render(inner, buf);
    }
}
