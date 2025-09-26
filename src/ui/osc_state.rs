use std::ops::RangeInclusive;
use std::sync::{Arc, Mutex};

use once_cell::sync::Lazy;
use ratatui::{layout::Rect, text::Line};
use unicode_width::UnicodeWidthChar;

use crate::ui::span::{LinkMeta, SpanKind};

#[derive(Clone, Debug)]
pub struct OscSpan {
    pub href: Arc<LinkMeta>,
    pub y: u16,
    pub x_range: RangeInclusive<u16>,
}

#[derive(Clone, Debug, Default)]
pub struct OscRenderState {
    pub spans: Vec<OscSpan>,
}

static OSC_RENDER_STATE: Lazy<Mutex<OscRenderState>> =
    Lazy::new(|| Mutex::new(OscRenderState::default()));

pub fn set_render_state(state: OscRenderState) {
    if let Ok(mut guard) = OSC_RENDER_STATE.lock() {
        *guard = state;
    }
}

pub fn take_render_state() -> OscRenderState {
    OSC_RENDER_STATE
        .lock()
        .map(|state| state.clone())
        .unwrap_or_default()
}

pub fn compute_render_state(
    area: Rect,
    lines: &[Line<'static>],
    metadata: &[Vec<SpanKind>],
    vertical_offset: usize,
    horizontal_offset: u16,
) -> OscRenderState {
    if area.width == 0 || area.height == 0 {
        return OscRenderState::default();
    }

    let mut spans: Vec<OscSpan> = Vec::new();
    let start_col = horizontal_offset as usize;
    let area_width = area.width as usize;
    let end_col = start_col + area_width;

    for row in 0..area.height as usize {
        let line_index = vertical_offset + row;
        if line_index >= lines.len() {
            break;
        }
        let line = &lines[line_index];
        let kinds = metadata.get(line_index);
        let y = area.y + row as u16;

        struct Run {
            meta: Arc<LinkMeta>,
            start_x: Option<u16>,
            end_x: Option<u16>,
        }

        let mut active_run: Option<Run> = None;
        let mut absolute_col = 0usize;

        for (span_idx, span) in line.spans.iter().enumerate() {
            let span_kind = kinds
                .and_then(|k| k.get(span_idx))
                .cloned()
                .unwrap_or(SpanKind::Text);

            for ch in span.content.chars() {
                let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
                if ch_width == 0 {
                    continue;
                }

                let char_start = absolute_col;
                let char_end = absolute_col + ch_width;
                let visible_start = char_start.max(start_col);
                let visible_end = char_end.min(end_col);
                let is_visible = visible_start < visible_end;

                match span_kind.link_meta() {
                    Some(meta) => {
                        let same_run = active_run
                            .as_ref()
                            .map(|run| run.meta.href() == meta.href())
                            .unwrap_or(false);
                        if !same_run {
                            if let Some(run) = active_run.take() {
                                if let (Some(start_x), Some(end_x)) = (run.start_x, run.end_x) {
                                    spans.push(OscSpan {
                                        href: run.meta,
                                        y,
                                        x_range: start_x..=end_x,
                                    });
                                }
                            }
                            active_run = Some(Run {
                                meta: Arc::new(meta.clone()),
                                start_x: None,
                                end_x: None,
                            });
                        }

                        if let Some(run) = active_run.as_mut() {
                            if is_visible {
                                let first_cell = area.x + (visible_start - start_col) as u16;
                                let last_cell = area.x + (visible_end - start_col - 1) as u16;
                                if run.start_x.is_none() {
                                    run.start_x = Some(first_cell);
                                }
                                run.end_x = Some(last_cell);
                            }
                        }
                    }
                    None => {
                        if let Some(run) = active_run.take() {
                            if let (Some(start_x), Some(end_x)) = (run.start_x, run.end_x) {
                                spans.push(OscSpan {
                                    href: run.meta,
                                    y,
                                    x_range: start_x..=end_x,
                                });
                            }
                        }
                    }
                }

                absolute_col = char_end;
            }
        }

        if let Some(run) = active_run {
            if let (Some(start_x), Some(end_x)) = (run.start_x, run.end_x) {
                spans.push(OscSpan {
                    href: run.meta,
                    y,
                    x_range: start_x..=end_x,
                });
            }
        }
    }

    OscRenderState { spans }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::{Line, Span};

    #[test]
    fn compute_render_state_tracks_visible_link_segment() {
        let line = Line::from(vec![Span::raw("Visit "), Span::raw("Rust")]);
        let metadata = vec![SpanKind::Text, SpanKind::link("https://www.rust-lang.org")];
        let area = Rect::new(0, 0, 20, 1);
        let state = compute_render_state(area, &[line], &[metadata], 0, 0);
        assert_eq!(state.spans.len(), 1);
        let span = &state.spans[0];
        assert_eq!(*span.x_range.start(), 6);
        assert_eq!(*span.x_range.end(), 9);
        assert_eq!(span.href.href(), "https://www.rust-lang.org");
    }
}
