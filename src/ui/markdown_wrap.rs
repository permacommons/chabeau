use crate::ui::span::SpanKind;
use ratatui::{style::Style, text::Span};
use unicode_width::UnicodeWidthStr;

/// Wrap spans to the provided width while preserving styles and word boundaries.
/// Shared between markdown rendering and range computation so downstream
/// consumers stay in sync.
pub(crate) fn wrap_spans_to_width_generic_shared(
    spans: &[(Span<'static>, SpanKind)],
    max_width: usize,
) -> Vec<Vec<(Span<'static>, SpanKind)>> {
    const MAX_UNBREAKABLE_LENGTH: usize = 30;
    if spans.is_empty() {
        return vec![Vec::new()];
    }
    let mut wrapped_lines = Vec::new();
    let mut current_line: Vec<(Span<'static>, SpanKind)> = Vec::new();
    let mut current_width = 0usize;
    // Break incoming spans into owned (text, style) parts
    let mut parts: Vec<(String, Style, SpanKind)> = spans
        .iter()
        .map(|(s, kind)| (s.content.to_string(), s.style, *kind))
        .collect();
    for (mut text, style, kind) in parts.drain(..) {
        while !text.is_empty() {
            let mut chars_to_fit = 0usize;
            let mut width_so_far = 0usize;
            let mut last_break_pos: Option<(usize, usize)> = None;
            for (char_pos, ch) in text.char_indices() {
                let cw = UnicodeWidthStr::width(ch.encode_utf8(&mut [0; 4]));
                if current_width + width_so_far + cw <= max_width {
                    width_so_far += cw;
                    chars_to_fit = char_pos + ch.len_utf8();
                    if ch.is_whitespace() {
                        last_break_pos = Some((char_pos + ch.len_utf8(), width_so_far));
                    }
                } else {
                    break;
                }
            }
            if chars_to_fit == 0 {
                // Nothing fits on this line
                if !current_line.is_empty() {
                    wrapped_lines.push(std::mem::take(&mut current_line));
                    current_width = 0;
                    continue;
                } else {
                    // Consider unbreakable word
                    let next_word_end = text.find(char::is_whitespace).unwrap_or(text.len());
                    let next_word = &text[..next_word_end];
                    let ww = UnicodeWidthStr::width(next_word);
                    if ww <= MAX_UNBREAKABLE_LENGTH {
                        current_line.push((Span::styled(next_word.to_string(), style), kind));
                        current_width += ww;
                        if next_word_end < text.len() {
                            text = text[next_word_end..].to_string();
                        } else {
                            break;
                        }
                    } else {
                        // Hard break the very long token
                        let mut forced_width = 0usize;
                        let mut forced_end = text.len();
                        for (char_pos, ch) in text.char_indices() {
                            let cw = UnicodeWidthStr::width(ch.encode_utf8(&mut [0; 4]));
                            if forced_width + cw > max_width {
                                forced_end = char_pos;
                                break;
                            }
                            forced_width += cw;
                        }
                        if forced_end > 0 {
                            let chunk = text[..forced_end].to_string();
                            current_line.push((Span::styled(chunk, style), kind));
                            current_width = forced_width;
                            text = text[forced_end..].to_string();
                            if !text.is_empty() {
                                wrapped_lines.push(std::mem::take(&mut current_line));
                                current_width = 0;
                            }
                        } else {
                            current_line.push((Span::styled(text.clone(), style), kind));
                            current_width += UnicodeWidthStr::width(text.as_str());
                            break;
                        }
                    }
                }
            } else if chars_to_fit >= text.len() {
                current_line.push((Span::styled(text.clone(), style), kind));
                current_width += width_so_far;
                break;
            } else {
                let (break_pos, _bw) = last_break_pos.unwrap_or((chars_to_fit, width_so_far));
                if last_break_pos.is_none() && current_width > 0 {
                    // No natural break inside the incoming span; start it on the next line so
                    // multi-word links and long tokens stay intact.
                    wrapped_lines.push(std::mem::take(&mut current_line));
                    current_width = 0;
                    continue;
                }
                let left = text[..break_pos].trim_end();
                if !left.is_empty() {
                    let left_width = UnicodeWidthStr::width(left);
                    if current_width > 0 && current_width + left_width > max_width {
                        wrapped_lines.push(std::mem::take(&mut current_line));
                        current_width = 0;
                    }
                    if left_width > 0 {
                        current_line.push((Span::styled(left.to_string(), style), kind));
                        current_width += left_width;
                    }
                }
                text = text[break_pos..].trim_start().to_string();
                if !text.is_empty() {
                    wrapped_lines.push(std::mem::take(&mut current_line));
                    current_width = 0;
                }
            }
        }
    }
    if !current_line.is_empty() {
        wrapped_lines.push(current_line);
    }
    if wrapped_lines.is_empty() {
        vec![Vec::new()]
    } else {
        wrapped_lines
    }
}

#[cfg(test)]
mod tests {
    use super::wrap_spans_to_width_generic_shared;
    use crate::ui::span::SpanKind;
    use ratatui::text::Span;

    #[test]
    fn wrap_splits_at_spaces() {
        let spans = vec![(Span::raw("word boundary test"), SpanKind::Text)];
        let wrapped = wrap_spans_to_width_generic_shared(&spans, 6);
        let lines: Vec<String> = wrapped
            .into_iter()
            .map(|line| {
                line.iter()
                    .map(|(s, _)| s.content.as_ref())
                    .collect::<String>()
            })
            .filter(|s| !s.is_empty())
            .collect();
        assert_eq!(lines, vec!["word", "bounda", "ry", "test"]);
    }
}
