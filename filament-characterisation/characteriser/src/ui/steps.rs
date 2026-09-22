use crate::{steps::Section, style::BLOCK_TITLE_STYLE};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Styled,
    widgets::{Block, Padding, Paragraph, StatefulWidget, Widget},
};

/// Scroll position of the step list.
#[derive(Debug)]
pub struct StepsState {
    /// If the view follows the end of the list as steps are added.
    follow: bool,

    /// The height of the last viewport drawn, so a page scroll knows what a
    /// page is.
    height: usize,

    /// The index of the first visible line.
    offset: usize,
}

impl Default for StepsState {
    /// Note that this can't be derived: following is on by default, but
    /// `bool::default()` is `false`.
    fn default() -> Self {
        Self {
            follow: true,
            height: 0,
            offset: 0,
        }
    }
}

impl StepsState {
    /// Scrolls to the first line.
    pub fn scroll_to_top(&mut self) {
        self.follow = false;
        self.offset = 0;
    }

    /// Scrolls to the last line and follows it again.
    pub fn scroll_to_bottom(&mut self) {
        self.follow = true;
    }

    /// Scrolls `lines` lines towards the end of the list.
    pub fn scroll_down(&mut self, lines: usize) {
        self.offset = self.offset.saturating_add(lines);
    }

    /// Scrolls `lines` lines towards the start of the list.
    ///
    /// Any upward scroll stops the view following the end of the list.
    pub fn scroll_up(&mut self, lines: usize) {
        self.follow = false;
        self.offset = self.offset.saturating_sub(lines);
    }

    /// The height of the last viewport drawn, for a page scroll.
    pub fn page(&self) -> usize {
        self.height.max(1)
    }
}

/// The scrollable list of steps.
pub struct Steps<'a> {
    pub root: &'a Section,
}

impl StatefulWidget for Steps<'_> {
    type State = StepsState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let block = Block::bordered()
            .padding(Padding::symmetric(2, 0))
            .title("Steps".set_style(BLOCK_TITLE_STYLE));
        let inner = block.inner(area);
        block.render(area, buf);

        // The lines are rebuilt every frame. That's fine: ratatui diffs
        // buffers, so an unchanged frame costs no terminal I/O, and a running
        // step's spinner means most frames change anyway.
        let lines = self.root.render_children(inner.width);
        let height = inner.height as usize;
        state.height = height;

        // A pending step must always be visible. It's always the last step in
        // the tree and always a leaf, so it's always the last line — which
        // makes "scroll to it" the same as following the end of the list.
        if self.root.pending().is_some() {
            state.follow = true;
        }

        // Clamp every frame, since the list grows and the terminal can be
        // resized under it.
        let last = lines.len().saturating_sub(height);
        if state.follow {
            state.offset = last;
        } else {
            state.offset = state.offset.min(last);

            // Scrolling back to the bottom starts following again.
            if state.offset == last {
                state.follow = true;
            }
        }

        // No `Wrap`: the lines are already fitted to the width, and without it
        // ratatui truncates rather than reflows anything that still overruns —
        // a backstop, not the mechanism.
        let end = (state.offset + height).min(lines.len());
        let visible = lines[state.offset..end].to_vec();
        Paragraph::new(visible).render(inner, buf);
    }
}
