//! The tree of steps the procedure emits and the application renders.
//!
//! Note: don't import `ratatui::text::Text` here — it collides with
//! `StepKind::Text`. Only `Line` and `Span` are needed.

use crate::{
    style::{CONFIRMATION_STYLE, ERROR_STYLE, VARIABLE_STYLE},
    ui::format_duration,
};
use ratatui::text::{Line, Span};
use std::{
    fmt::Display,
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

/// The columns each level of nesting indents by.
const INDENT: u16 = 2;

/// The position of a step in the tree, as child indices from the root.
///
/// Paths stay valid for the life of the run because steps are only ever
/// appended — never removed, never reordered.
// TODO: remove this once `Context` addresses steps by path.
#[allow(dead_code)]
pub type StepPath = Vec<usize>;

/// A single step in the procedure.
#[derive(Debug)]
pub struct Step {
    /// When the step finished, if it has.
    pub finished_at: Option<Instant>,

    pub kind: StepKind,

    pub started_at: Instant,

    pub status: StepStatus,
}

/// What a step displays and how the user interacts with it.
#[derive(Debug)]
pub enum StepKind {
    /// A gate: something the user has to make true, e.g. "Confirm that the
    /// roughing pump is running", before the procedure continues.
    ///
    /// Not a yes/no question. There's no answer to record — the step's status
    /// and `finished_at` say that it was passed and when — and no way to refuse:
    /// a user who can't make it true quits instead.
    Confirm {
        prompt: String,

        /// Taken by the application when the user confirms.
        ///
        /// `Some` exactly while this step is waiting for input, which is what
        /// `Section::pending_mut` looks for.
        responder: Option<oneshot::Sender<()>>,
    },

    /// A value typed by the user.
    Input {
        /// What the user has typed so far.
        buffer: String,

        /// Set when the buffer failed to parse, and shown beside the prompt.
        error: Option<String>,

        prompt: String,

        /// Taken by the application when the user submits a value.
        responder: Option<oneshot::Sender<String>>,

        /// The unit shown after the input field, e.g. "A".
        unit: Option<String>,

        /// The accepted value, set by the procedure once parsed.
        value: Option<String>,
    },

    /// A group of steps.
    Section(Section),

    /// One line of styled text: a remark, a measurement, an error, or something
    /// still in progress.
    ///
    /// These differ only in their spans and their status, so they are one kind
    /// with three constructors. A step that is still `Running` renders with a
    /// spinner and a live elapsed time, which is what makes it a "waiting"
    /// step; it stops the moment the procedure finishes it.
    ///
    /// `Vec<Span>` rather than a `Line` because the renderer prepends the
    /// indentation and status glyph and assembles the `Line` itself; a `Line`
    /// would carry its own alignment and style that the assembly would have to
    /// reconcile.
    Text { spans: Vec<Span<'static>> },
}

impl StepKind {
    /// A fatal error, styled with `ERROR_STYLE`.
    ///
    /// The step's `status` carries the failure; this is only its appearance.
    pub fn error(message: impl Into<String>) -> Self {
        Self::Text {
            spans: vec![Span::styled(message.into(), ERROR_STYLE)],
        }
    }

    /// A labelled value, e.g. "Forward voltage: 9.61 ± 0.03 mV".
    ///
    /// The label is rendered in the default style and the value in
    /// `VARIABLE_STYLE`, the two-tone idiom the vacuum control binary uses for
    /// every reading. It's what the operator's eye scans for, and where the `±`
    /// uncertainty goes.
    pub fn measurement(label: impl Into<String>, value: impl Display) -> Self {
        Self::Text {
            spans: vec![
                Span::raw(format!("{}: ", label.into())),
                Span::styled(value.to_string(), VARIABLE_STYLE),
            ],
        }
    }

    /// A plain line of text.
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text {
            spans: vec![Span::raw(text.into())],
        }
    }
}

/// A group of steps with a title.
///
/// `Default` gives the root of the tree: no children and no title, because the
/// UI renders the root's children rather than the root itself.
#[derive(Debug, Default)]
pub struct Section {
    /// The steps nested beneath this one, in the order they began.
    pub children: Vec<Step>,

    pub title: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum StepStatus {
    Done,
    Failed,
    #[default]
    Running,
}

impl Step {
    /// A step that started now and hasn't finished.
    pub fn new(kind: StepKind) -> Self {
        Self {
            finished_at: None,
            kind,
            started_at: Instant::now(),
            status: StepStatus::default(),
        }
    }

    /// How long the step ran for, or has been running.
    pub fn elapsed(&self) -> Duration {
        self.finished_at
            .unwrap_or_else(Instant::now)
            .duration_since(self.started_at)
    }

    /// Whether the step is waiting on the user.
    ///
    /// True exactly while it still holds a responder: taking the responder is
    /// what ends the wait.
    pub fn is_pending(&self) -> bool {
        match &self.kind {
            StepKind::Confirm { responder, .. } => responder.is_some(),
            StepKind::Input { responder, .. } => responder.is_some(),
            StepKind::Section(_) | StepKind::Text { .. } => false,
        }
    }

    /// The step as rendered lines.
    ///
    /// Text is never wrapped: a step produces a fixed number of lines — one for
    /// itself, plus whatever its children produce — and anything too wide is
    /// ellipsified to fit. That's what makes scrolling a slice index: the line
    /// count doesn't depend on the width.
    ///
    /// `width` is needed anyway, because a line with a right-hand element — a
    /// section's total elapsed time, a running step's elapsed seconds — lays it
    /// out against the right edge and ellipsifies the left part to fit, so the
    /// time stays visible however long the title is.
    ///
    /// A section renders its title and then calls this on each of its children
    /// with a width `INDENT` columns narrower, indenting what comes back — so
    /// the recursion carries the nesting and no depth parameter is needed.
    ///
    /// Note: this deliberately is not `Widget::render`. Do not implement
    /// `Widget` for `Step` — it would have to draw into a `Rect`, and then
    /// scrolling would need the oversized-buffer machinery instead of a slice.
    /// If a chart step is ever added, switch the whole list to that approach
    /// rather than special-casing one kind, and look at `tui-scrollview` before
    /// writing it by hand.
    pub fn render(&self, width: u16) -> Vec<Line<'static>> {
        // Style at the span level, never the line level, so that a section
        // prepending an indent span can't interact with a line-level style.
        let glyph = Span::raw(format!("{} ", self.glyph()));
        let available = width.saturating_sub(glyph.width() as u16);

        match &self.kind {
            StepKind::Confirm { prompt, responder } => {
                let mut spans = vec![Span::raw(prompt.clone())];

                // Once it's passed there's nothing to add: the glyph says so.
                if responder.is_some() {
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled("[Enter] Confirm", CONFIRMATION_STYLE));
                }

                vec![line(glyph, ellipsify(spans, available))]
            }

            StepKind::Input {
                buffer,
                error,
                prompt,
                responder,
                unit,
                value,
            } => {
                let mut spans = vec![Span::raw(format!("{}: ", prompt))];
                if responder.is_some() {
                    // A block cursor, since the terminal's own cursor is hidden.
                    spans.push(Span::styled(format!("{}█", buffer), CONFIRMATION_STYLE));
                } else if let Some(value) = value {
                    spans.push(Span::styled(value.clone(), VARIABLE_STYLE));
                }
                if let Some(unit) = unit {
                    spans.push(Span::raw(format!(" {}", unit)));
                }
                if let Some(error) = error {
                    spans.push(Span::styled(format!("  {}", error), ERROR_STYLE));
                }
                vec![line(glyph, ellipsify(spans, available))]
            }

            StepKind::Section(section) => {
                let elapsed = format_duration(self.elapsed());
                let title = Span::styled(section.title.clone(), crate::style::BLOCK_TITLE_STYLE);
                let mut lines = vec![line(glyph, right_align(vec![title], &elapsed, available))];

                // Narrowing the width before rendering and indenting after
                // means a nested section's right-aligned elapsed time still
                // lands in the correct column once the indent is prepended.
                lines.extend(
                    section
                        .render_children(width.saturating_sub(INDENT))
                        .into_iter()
                        .map(indent),
                );
                lines
            }

            StepKind::Text { spans } => {
                let spans = spans.clone();
                if self.status == StepStatus::Running {
                    let elapsed = format_duration(self.elapsed());
                    vec![line(glyph, right_align(spans, &elapsed, available))]
                } else {
                    vec![line(glyph, ellipsify(spans, available))]
                }
            }
        }
    }

    /// The status glyph shown at the start of the step's line.
    fn glyph(&self) -> String {
        /// The frames of the spinner shown beside a running step.
        const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

        /// How long each spinner frame is shown for.
        const SPINNER_FRAME_INTERVAL: Duration = Duration::from_millis(80);

        match self.status {
            StepStatus::Done => String::from("✔"),
            StepStatus::Failed => String::from("✖"),

            // Indexed from the step's own elapsed time, so no tick counter has
            // to be passed down the tree.
            StepStatus::Running => {
                let frame = self.elapsed().as_millis() / SPINNER_FRAME_INTERVAL.as_millis();
                String::from(SPINNER_FRAMES[frame as usize % SPINNER_FRAMES.len()])
            }
        }
    }
}

impl Section {
    /// Appends a step and returns its index in `children`.
    // TODO: remove this once `Context` emits steps.
    #[allow(dead_code)]
    pub fn push(&mut self, kind: StepKind) -> usize {
        self.children.push(Step::new(kind));
        self.children.len() - 1
    }

    /// The step waiting on the user, if any.
    ///
    /// The procedure is sequential, so the step awaiting input can only be the
    /// last one in the tree: follow the last child down through nested sections
    /// and check whether what you land on still holds a responder. That's a
    /// walk of the tree's depth, not its size.
    ///
    /// IMPORTANT: if the procedure ever runs two sections concurrently, this
    /// assumption breaks and this has to become a depth-first search for a step
    /// holding a responder.
    pub fn pending(&self) -> Option<&Step> {
        let mut section = self;

        loop {
            match section.children.last() {
                Some(Step {
                    kind: StepKind::Section(child),
                    ..
                }) => section = child,
                Some(step) => return step.is_pending().then_some(step),
                None => return None,
            }
        }
    }

    /// The step waiting on the user, if any.
    ///
    /// See [`Section::pending`].
    pub fn pending_mut(&mut self) -> Option<&mut Step> {
        // Written as a loop rather than a recursion because the borrow checker
        // rejects the recursive form: returning `section.pending_mut()` from
        // one arm of a `match` on `step.kind` would require that borrow to last
        // as long as the return value, clashing with the arm that returns the
        // step itself. Matching on the value `last_mut` returns, and walking
        // the borrow down the tree, avoids holding two borrows at once.
        let mut section = self;

        loop {
            match section.children.last_mut() {
                Some(Step {
                    kind: StepKind::Section(child),
                    ..
                }) => section = child,
                Some(step) => return step.is_pending().then_some(step),
                None => return None,
            }
        }
    }

    /// The section's children as rendered lines, at this section's own level of
    /// indentation.
    ///
    /// Used by `Step::render` for nested sections, which indents what comes
    /// back, and by the UI for the root, whose own title is never shown and
    /// whose children aren't indented.
    pub fn render_children(&self, width: u16) -> Vec<Line<'static>> {
        self.children
            .iter()
            .flat_map(|child| child.render(width))
            .collect()
    }

    /// The section at `path`, if it exists and is a section.
    // TODO: remove this once `Context` emits steps.
    #[allow(dead_code)]
    pub fn section_at_mut(&mut self, path: &[usize]) -> Option<&mut Section> {
        match path.split_first() {
            None => Some(self),
            Some((index, rest)) => match &mut self.children.get_mut(*index)?.kind {
                StepKind::Section(section) => section.section_at_mut(rest),
                _ => None,
            },
        }
    }

    /// The step at `path`, if it exists.
    // TODO: remove this once `Context` emits steps.
    #[allow(dead_code)]
    pub fn step_at_mut(&mut self, path: &[usize]) -> Option<&mut Step> {
        let (index, rest) = path.split_first()?;
        let step = self.children.get_mut(*index)?;

        if rest.is_empty() {
            return Some(step);
        }

        match &mut step.kind {
            StepKind::Section(section) => section.step_at_mut(rest),
            _ => None,
        }
    }
}

/// Assembles a line from a step's glyph and its spans.
fn line(glyph: Span<'static>, spans: Vec<Span<'static>>) -> Line<'static> {
    let mut all = Vec::with_capacity(spans.len() + 1);
    all.push(glyph);
    all.extend(spans);
    Line::from(all)
}

/// Indents a line by one level of nesting.
fn indent(line: Line<'static>) -> Line<'static> {
    let mut spans = Vec::with_capacity(line.spans.len() + 1);
    spans.push(Span::raw(" ".repeat(INDENT as usize)));
    spans.extend(line.spans);
    Line::from(spans)
}

/// Lays `right` out against the right edge, ellipsifying `spans` to fit.
///
/// Never uses `Line::alignment`: ratatui would compute it against the drawing
/// area rather than the indented width a nested section renders into, so the
/// padding is added here instead.
fn right_align(spans: Vec<Span<'static>>, right: &str, width: u16) -> Vec<Span<'static>> {
    // One space so a full-width left part can't run into the right one.
    let right_width = right.chars().count() as u16 + 1;
    let mut spans = ellipsify(spans, width.saturating_sub(right_width));

    let used: usize = spans.iter().map(|span| span.width()).sum();
    let padding = (width as usize).saturating_sub(used + right.chars().count());
    spans.push(Span::raw(" ".repeat(padding)));
    spans.push(Span::styled(
        right.to_string(),
        crate::style::SUBTLE_TEXT_STYLE,
    ));
    spans
}

/// Truncates `spans` to `width` columns, ending the last surviving span with
/// `…` when anything was cut.
///
/// Everything is ellipsified rather than left to clip at the block edge, so a
/// truncated line always says so — silent truncation looks like a short line,
/// and a step list is exactly where a reader would fail to notice.
///
/// `Span::width` is unicode-aware, so accumulating it is exact. Cutting *within*
/// a span counts chars instead, which is correct for everything the
/// constructors generate and for the ASCII an operator types; a wide character
/// reaching this would need `unicode-width` to place the cut.
fn ellipsify(spans: Vec<Span<'static>>, width: u16) -> Vec<Span<'static>> {
    let width = width as usize;
    let mut kept: Vec<Span<'static>> = Vec::with_capacity(spans.len());
    let mut used = 0;

    for span in spans {
        let span_width = span.width();
        if used + span_width <= width {
            used += span_width;
            kept.push(span);
            continue;
        }

        // This span doesn't fit. Keep what does, less one column for the `…`.
        let available = width.saturating_sub(used).saturating_sub(1);
        if available > 0 {
            let content: String = span.content.chars().take(available).collect();
            kept.push(Span::styled(format!("{}…", content), span.style));
        } else if let Some(last) = kept.pop() {
            // Not even one column left, so replace the end of what we have.
            let content: String = last
                .content
                .chars()
                .take(last.content.chars().count().saturating_sub(1))
                .collect();
            kept.push(Span::styled(format!("{}…", content), last.style));
        }

        return kept;
    }

    kept
}

/// A hard-coded tree, so the widget can be exercised before the procedure
/// exists.
///
/// TODO: delete this once the procedure builds the tree (step 8 of the design
/// document's build order).
pub fn demo() -> Section {
    fn finished(kind: StepKind, status: StepStatus, seconds: u64) -> Step {
        Step {
            finished_at: Some(Instant::now()),
            kind,
            started_at: Instant::now() - Duration::from_secs(seconds),
            status,
        }
    }

    let mut root = Section::default();

    let mut preparing = Section {
        children: Vec::new(),
        title: String::from("Preparing"),
    };
    preparing.children.push(finished(
        StepKind::Input {
            buffer: String::new(),
            error: None,
            prompt: String::from("Filament"),
            responder: None,
            unit: None,
            value: Some(String::from("W-0007")),
        },
        StepStatus::Done,
        3,
    ));
    preparing.children.push(finished(
        StepKind::Confirm {
            prompt: String::from("Confirm that the chamber is sealed"),
            responder: None,
        },
        StepStatus::Done,
        5,
    ));
    preparing.children.push(finished(
        StepKind::text("Set the heating voltage limit to 30.000 V"),
        StepStatus::Done,
        1,
    ));
    root.children
        .push(finished(StepKind::Section(preparing), StepStatus::Done, 42));

    let mut pumping = Section {
        children: Vec::new(),
        title: String::from("Pumping down chamber"),
    };
    pumping.children.push(finished(
        StepKind::Confirm {
            prompt: String::from("Confirm that the roughing pump is running"),
            responder: None,
        },
        StepStatus::Done,
        4,
    ));
    pumping.children.push(finished(
        StepKind::text("Waiting for chamber to reach TMP operating pressure"),
        StepStatus::Done,
        187,
    ));
    pumping.children.push(finished(
        StepKind::text("Turned on the TMP"),
        StepStatus::Done,
        1,
    ));
    pumping.children.push(finished(
        StepKind::error("The TMP took too long to reach speed, retrying"),
        StepStatus::Failed,
        90,
    ));
    pumping.children.push(finished(
        StepKind::text("Waiting for TMP to reach speed"),
        StepStatus::Done,
        121,
    ));
    pumping.children.push(finished(
        StepKind::measurement("Base pressure", "2.4e-06 mbar"),
        StepStatus::Done,
        0,
    ));
    root.children
        .push(finished(StepKind::Section(pumping), StepStatus::Done, 365));

    let mut cold = Section {
        children: Vec::new(),
        title: String::from("Measuring cold resistance"),
    };
    for (index, polarity) in ["Forward", "Reverse"].iter().enumerate() {
        let mut sweep = Section {
            children: Vec::new(),
            title: format!("{} polarity", polarity),
        };
        for sample in 1..=6 {
            sweep.children.push(finished(
                StepKind::measurement(
                    format!("Sample {}", sample),
                    format!("{:.1} mV", 9.4 + sample as f64 * 0.07 + index as f64 * 0.2),
                ),
                StepStatus::Done,
                0,
            ));
        }
        sweep.children.push(finished(
            StepKind::measurement(
                format!("{} voltage", polarity),
                format!("{:.2} ± 0.03 mV", 9.61 + index as f64 * 0.2),
            ),
            StepStatus::Done,
            0,
        ));
        cold.children
            .push(finished(StepKind::Section(sweep), StepStatus::Done, 14));
    }
    cold.children.push(finished(
        StepKind::measurement("Cold resistance", "0.412 ± 0.002 Ω"),
        StepStatus::Done,
        0,
    ));
    root.children
        .push(finished(StepKind::Section(cold), StepStatus::Done, 31));

    let mut sweeping = Section {
        children: Vec::new(),
        title: String::from("Sweeping heating current"),
    };
    for point in 1..=9 {
        let current = point as f64 * 0.185;
        sweeping.children.push(finished(
            StepKind::measurement(
                format!("{:.3} A", current),
                format!("{:.1} mV", current * 412.0),
            ),
            StepStatus::Done,
            2,
        ));
    }
    sweeping.children.push(Step::new(StepKind::text(
        "Settling at 1.850 A before taking the next point",
    )));

    // The receiver is dropped: nothing is waiting on this confirmation, but the
    // sender being `Some` is what makes the step pending.
    let (responder, _) = oneshot::channel();
    sweeping.children.push(Step::new(StepKind::Confirm {
        prompt: String::from("Confirm that the filament current has settled"),
        responder: Some(responder),
    }));

    root.children.push(Step::new(StepKind::Section(sweeping)));

    root
}
