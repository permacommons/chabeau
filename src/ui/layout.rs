use std::collections::VecDeque;

use ratatui::text::Line;

use super::span::SpanKind;
use super::theme::Theme;
use crate::core::message::Message;

/// Policy for how tables should behave when they cannot reasonably fit within the terminal width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableOverflowPolicy {
    /// Try to wrap cells according to balanced column widths. Borders must remain intact.
    WrapCells,
}

/// Layout configuration used by the unified layout engine.
#[derive(Clone, Debug)]
pub struct LayoutConfig {
    pub width: Option<usize>,
    pub markdown_enabled: bool,
    pub syntax_enabled: bool,
    pub table_overflow_policy: TableOverflowPolicy,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            width: None,
            markdown_enabled: true,
            syntax_enabled: true,
            table_overflow_policy: TableOverflowPolicy::WrapCells,
        }
    }
}

/// Mapping for a single message's contribution to the flattened line stream.
#[derive(Clone, Debug, Default)]
pub struct MessageLineSpan {
    pub start: usize,
    pub len: usize,
}

/// Result of a layout pass. Carries the flattened lines along with metadata
/// describing each message's contribution and any code block ranges (for
/// selection/highlight overlays).
#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub lines: Vec<Line<'static>>,
    #[allow(dead_code)]
    pub span_metadata: Vec<Vec<SpanKind>>,
    pub message_spans: Vec<MessageLineSpan>,
    pub codeblock_ranges: Vec<(usize, usize, String)>,
}

pub struct LayoutEngine;

impl LayoutEngine {
    /// Convenience helper to layout plain-text messages with an explicit width.
    /// This applies width-aware wrapping consistently with the markdown path.
    pub fn layout_plain_text(
        messages: &VecDeque<Message>,
        theme: &Theme,
        width: Option<usize>,
        syntax_enabled: bool,
    ) -> Layout {
        let cfg = LayoutConfig {
            width,
            markdown_enabled: false,
            syntax_enabled,
            table_overflow_policy: TableOverflowPolicy::WrapCells,
        };
        Self::layout_messages(messages, theme, &cfg)
    }

    /// Perform a layout pass over the messages using the supplied theme and configuration.
    /// This is the single, width-aware pipeline that downstream systems (renderer, scroll math)
    /// should consume. No additional wrapping should be performed after this step.
    pub fn layout_messages(
        messages: &VecDeque<Message>,
        theme: &Theme,
        cfg: &LayoutConfig,
    ) -> Layout {
        if cfg.markdown_enabled {
            // Route through the existing markdown renderer with explicit width when provided.
            let mut lines = Vec::new();
            let mut span_metadata = Vec::new();
            let mut message_spans = Vec::with_capacity(messages.len());
            let mut codeblock_ranges = Vec::new();
            for msg in messages {
                let start = lines.len();
                let rendered = crate::ui::markdown::render_message_markdown_details_with_policy(
                    msg,
                    theme,
                    cfg.syntax_enabled,
                    cfg.width,
                    cfg.table_overflow_policy,
                );
                let crate::ui::markdown::RenderedMessageDetails {
                    lines: mut msg_lines,
                    codeblock_ranges: msg_ranges,
                    span_metadata: msg_meta,
                } = rendered;
                let msg_metadata = msg_meta.unwrap_or_else(|| {
                    msg_lines
                        .iter()
                        .map(|line| vec![SpanKind::Text; line.spans.len()])
                        .collect()
                });
                let len = msg_lines.len();
                span_metadata.extend(msg_metadata);
                lines.append(&mut msg_lines);
                message_spans.push(MessageLineSpan { start, len });
                for (offset, cb_len, content) in msg_ranges {
                    codeblock_ranges.push((start + offset, cb_len, content));
                }
            }
            Layout {
                lines,
                span_metadata,
                message_spans,
                codeblock_ranges,
            }
        } else {
            // Plain text fallback (no markdown). Build base lines/spans, then apply optional
            // width-aware wrapping per message so the layout stays aligned with rendering.
            let (base_lines, base_spans) =
                crate::ui::markdown::build_plain_display_lines_with_spans(messages, theme);
            if let Some(w) = cfg.width {
                let mut lines: Vec<Line<'static>> = Vec::new();
                let mut span_metadata: Vec<Vec<SpanKind>> = Vec::new();
                let mut spans: Vec<MessageLineSpan> = Vec::with_capacity(base_spans.len());
                for span in &base_spans {
                    let slice = &base_lines[span.start..span.start + span.len];
                    let wrapped =
                        crate::utils::scroll::ScrollCalculator::prewrap_lines(slice, w as u16);
                    let start = lines.len();
                    let len = wrapped.len();
                    for line in wrapped {
                        span_metadata.push(vec![SpanKind::Text; line.spans.len()]);
                        lines.push(line);
                    }
                    spans.push(MessageLineSpan { start, len });
                }
                Layout {
                    lines,
                    span_metadata,
                    message_spans: spans,
                    codeblock_ranges: Vec::new(),
                }
            } else {
                let span_metadata = base_lines
                    .iter()
                    .map(|line| vec![SpanKind::Text; line.spans.len()])
                    .collect();
                Layout {
                    lines: base_lines,
                    span_metadata,
                    message_spans: base_spans,
                    codeblock_ranges: Vec::new(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LayoutConfig, LayoutEngine, SpanKind, Theme};
    use crate::core::message::Message;
    use std::collections::VecDeque;

    #[test]
    fn markdown_layout_populates_span_metadata() {
        let mut messages = VecDeque::new();
        messages.push_back(Message {
            role: "assistant".into(),
            content: "Testing a [link](https://example.com) span.".into(),
        });
        let theme = Theme::dark_default();
        let layout = LayoutEngine::layout_messages(&messages, &theme, &LayoutConfig::default());

        assert_eq!(layout.lines.len(), layout.span_metadata.len());
        let mut saw_link = false;
        for kinds in &layout.span_metadata {
            if kinds.iter().any(|k| *k == SpanKind::Link) {
                saw_link = true;
                break;
            }
        }
        assert!(saw_link, "expected at least one link span kind");
    }

    #[test]
    fn plain_text_layout_synthesizes_metadata() {
        let mut messages = VecDeque::new();
        messages.push_back(Message {
            role: "user".into(),
            content: "Hello there".into(),
        });
        let theme = Theme::dark_default();
        let layout = LayoutEngine::layout_plain_text(&messages, &theme, Some(10), false);

        assert_eq!(layout.lines.len(), layout.span_metadata.len());
        for kinds in &layout.span_metadata {
            assert!(kinds.iter().all(|k| *k == SpanKind::Text));
        }
    }
}
