use crate::ui::span::SpanKind;
use ratatui::text::Line;

/// Description of a rendered message (line-based), used by the TUI renderer.
pub struct RenderedMessage {
    pub lines: Vec<Line<'static>>,
}

/// Extended render metadata used by the layout engine when downstream consumers
/// need per-message spans.
pub struct RenderedMessageDetails {
    pub lines: Vec<Line<'static>>,
    pub span_metadata: Option<Vec<Vec<SpanKind>>>,
}

impl RenderedMessageDetails {
    pub fn into_rendered(self) -> RenderedMessage {
        RenderedMessage { lines: self.lines }
    }
}
