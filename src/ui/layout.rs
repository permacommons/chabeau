use std::collections::VecDeque;

use ratatui::text::Line;

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

/// Result of a layout pass. For now this is a thin wrapper over the flattened Lines,
/// but it can be extended with block metadata, per-line widths, and coordinate mappings.
#[derive(Clone, Debug)]
pub struct Layout {
    pub lines: Vec<Line<'static>>,
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
            let mut out = Vec::new();
            for msg in messages {
                let rendered = crate::ui::markdown::render_message_markdown_with_policy(
                    msg,
                    theme,
                    cfg.syntax_enabled,
                    cfg.width,
                    cfg.table_overflow_policy,
                );
                out.extend(rendered.lines);
            }
            Layout { lines: out }
        } else {
            // Plain text fallback (no markdown). Build plain lines, then wrap to width if provided
            // so long lines do not overflow when markdown is disabled.
            let mut lines = crate::ui::markdown::build_plain_display_lines(messages, theme);
            if let Some(w) = cfg.width {
                lines = crate::utils::scroll::ScrollCalculator::prewrap_lines(&lines, w as u16);
            }
            Layout { lines }
        }
    }
}
