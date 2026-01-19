use crate::ui::span::SpanKind;
use ratatui::{style::Style, text::Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Wrap spans to the provided width while preserving styles and word boundaries.
/// Shared between markdown rendering and range computation so downstream
/// consumers stay in sync. A continuation indent width can be supplied to
/// account for prefixes added after wrapping (e.g., hanging indents).
pub(crate) fn wrap_spans_to_width_generic_shared(
    spans: &[(Span<'static>, SpanKind)],
    max_width: usize,
    continuation_indent_width: usize,
) -> Vec<Vec<(Span<'static>, SpanKind)>> {
    const MAX_UNBREAKABLE_LENGTH: usize = 30;
    if spans.is_empty() {
        return vec![Vec::new()];
    }
    let mut wrapped_lines = Vec::new();
    let mut current_line: Vec<(Span<'static>, SpanKind)> = Vec::new();
    let mut current_width = 0usize;
    let continuation_width = max_width
        .saturating_sub(continuation_indent_width)
        .max(1 /* prevent zero width */);
    let mut line_limit = max_width;
    // Break incoming spans into owned (text, style) parts
    let mut parts: Vec<(String, Style, SpanKind)> = spans
        .iter()
        .map(|(s, kind)| (s.content.to_string(), s.style, kind.clone()))
        .collect();
    for (mut text, style, kind) in parts.drain(..) {
        while !text.is_empty() {
            let mut chars_to_fit = 0usize;
            let mut width_so_far = 0usize;
            let mut last_break_pos: Option<(usize, usize)> = None;
            for (grapheme_start, grapheme) in text.grapheme_indices(true) {
                let grapheme_end = grapheme_start + grapheme.len();
                let gw = UnicodeWidthStr::width(grapheme);
                if current_width + width_so_far + gw <= line_limit {
                    width_so_far += gw;
                    chars_to_fit = grapheme_end;
                    if grapheme.chars().all(|c| c.is_whitespace()) {
                        last_break_pos = Some((grapheme_end, width_so_far));
                    }
                } else {
                    break;
                }
            }
            if chars_to_fit == 0 {
                // Nothing fits on this line
                if !current_line.is_empty() {
                    // Look back at the previous span to find natural word boundary.
                    // When styled text (code, emphasis) fills the line and the next span
                    // starts with whitespace/punctuation, we want to recognize that as
                    // a word boundary between the spans, not as content of the next span.
                    if let Some((_last_span, _)) = current_line.last() {
                        // Extract any leading punctuation to keep with previous line.
                        // Trim any whitespace. Never lose characters.
                        // Examples: ") today" → extract ")", trim " ", wrap "today"
                        //           "))) more" → extract ")))", trim " ", wrap "more"
                        //           " useful" → trim " ", wrap "useful"
                        //           " )" → trim " ", wrap ")" (standalone, don't backtrack)
                        let mut punct_start = None; // Where punctuation begins (None if not found)
                        let mut punct_end = 0;
                        let mut ws_end = 0;

                        for (idx, ch) in text.char_indices() {
                            if punct_start.is_none() {
                                if ch.is_whitespace() {
                                    ws_end = idx + ch.len_utf8();
                                } else if !ch.is_alphanumeric() && ch != '_' {
                                    // Found first punctuation character
                                    punct_start = Some(idx);
                                    punct_end = idx + ch.len_utf8();
                                    ws_end = punct_end;
                                } else {
                                    // Hit word character, stop
                                    break;
                                }
                            } else {
                                // We already found punctuation, continue scanning
                                if ch.is_whitespace() {
                                    ws_end = idx + ch.len_utf8();
                                } else if !ch.is_alphanumeric() && ch != '_' {
                                    // Found additional punctuation character
                                    punct_end = idx + ch.len_utf8();
                                    ws_end = punct_end;
                                } else {
                                    // Hit word character, stop
                                    break;
                                }
                            }
                        }

                        // Check if punctuation fits on current line
                        if punct_end > 0 {
                            let punct_width = UnicodeWidthStr::width(&text[..punct_end]);
                            if current_width + punct_width <= line_limit {
                                // Punctuation fits, add it to current line and trim trailing space
                                let punct_text = text[..punct_end].to_string();
                                current_line.push((Span::styled(punct_text, style), kind.clone()));
                                // Note: current_width update not needed - will be reset when wrapping
                                text = text[ws_end..].to_string();

                                if text.is_empty() {
                                    // After processing boundary characters, nothing remains
                                    wrapped_lines.push(std::mem::take(&mut current_line));
                                    current_width = 0;
                                    line_limit = continuation_width;
                                    continue;
                                }
                            } else if punct_start == Some(0) {
                                // Punctuation doesn't fit AND it's directly adjacent (no leading space).
                                // Before backtracking, check if we'd make progress:
                                // If styled span + punctuation won't fit on a new line, don't backtrack.
                                let punct_width = UnicodeWidthStr::width(&text[..punct_end]);
                                let last_span_width = current_line
                                    .last()
                                    .map(|(span, _)| UnicodeWidthStr::width(span.content.as_ref()))
                                    .unwrap_or(0);

                                // Only backtrack if styled word + punctuation will fit on continuation line
                                if last_span_width + punct_width <= continuation_width {
                                    // Backtrack: pop styled span, wrap current line, then add styled span
                                    // to next line followed by current text. This preserves the styling.
                                    if let Some((last_span, last_kind)) = current_line.pop() {
                                        // Wrap the current line (without the styled span)
                                        wrapped_lines.push(std::mem::take(&mut current_line));

                                        // Start new line with the styled span (preserving its style/kind)
                                        current_line.push((last_span, last_kind));
                                        current_width = UnicodeWidthStr::width(
                                            current_line[0].0.content.as_ref(),
                                        );
                                        line_limit = continuation_width;

                                        // Restart the while loop to reprocess current text on the new line
                                        continue;
                                    }
                                }
                                // else: backtracking won't help, let normal wrapping handle it
                            } else if punct_start.is_some() {
                                // Punctuation doesn't fit but "stands alone" (has leading space).
                                // Trim the leading space and let punctuation wrap normally.
                                text = text.trim_start().to_string();
                            }
                        } else if ws_end > 0 {
                            // Just whitespace, trim it
                            text = text[ws_end..].to_string();

                            if text.is_empty() {
                                // After trimming whitespace, nothing remains
                                wrapped_lines.push(std::mem::take(&mut current_line));
                                current_width = 0;
                                line_limit = continuation_width;
                                continue;
                            }
                        }
                    }

                    wrapped_lines.push(std::mem::take(&mut current_line));
                    current_width = 0;
                    line_limit = continuation_width;
                    continue;
                } else {
                    // Consider unbreakable word
                    let next_word_end = text.find(char::is_whitespace).unwrap_or(text.len());
                    let next_word = &text[..next_word_end];
                    let ww = UnicodeWidthStr::width(next_word);
                    if ww <= MAX_UNBREAKABLE_LENGTH {
                        current_line
                            .push((Span::styled(next_word.to_string(), style), kind.clone()));
                        current_width += ww;
                        if next_word_end < text.len() {
                            text = text[next_word_end..].to_string();
                        } else {
                            break;
                        }
                    } else {
                        // Hard break the very long token
                        let mut forced_width = 0usize;
                        let mut forced_end = 0usize;
                        for (grapheme_start, grapheme) in text.grapheme_indices(true) {
                            let grapheme_end = grapheme_start + grapheme.len();
                            let gw = UnicodeWidthStr::width(grapheme);
                            if forced_width + gw > line_limit {
                                if forced_end == 0 {
                                    forced_end = grapheme_end;
                                    forced_width += gw;
                                }
                                break;
                            }
                            forced_width += gw;
                            forced_end = grapheme_end;
                        }
                        if forced_end == 0 {
                            forced_end = text.len();
                            forced_width = UnicodeWidthStr::width(text.as_str());
                        }
                        let chunk = text[..forced_end].to_string();
                        current_line.push((Span::styled(chunk, style), kind.clone()));
                        current_width = forced_width;
                        text = text[forced_end..].to_string();
                        if !text.is_empty() {
                            wrapped_lines.push(std::mem::take(&mut current_line));
                            current_width = 0;
                            line_limit = continuation_width;
                        }
                    }
                }
            } else if chars_to_fit >= text.len() {
                current_line.push((Span::styled(text.clone(), style), kind.clone()));
                current_width += width_so_far;
                break;
            } else {
                let (break_pos, _bw) = last_break_pos.unwrap_or((chars_to_fit, width_so_far));
                if last_break_pos.is_none() && current_width > 0 {
                    // No natural break inside the incoming span; start it on the next line so
                    // multi-word links and long tokens stay intact.
                    // BUT FIRST: apply lookahead to find word boundaries between spans.
                    if let Some((_last_span, _)) = current_line.last() {
                        // Extract any leading punctuation to keep with previous line.
                        // Trim any whitespace. Never lose characters.
                        //           " )" → trim " ", wrap ")" (standalone, don't backtrack)
                        //           "))) more" → extract ")))", wrap "more"
                        let mut punct_start = None; // Where punctuation begins (None if not found)
                        let mut punct_end = 0;
                        let mut ws_end = 0;

                        for (idx, ch) in text.char_indices() {
                            if punct_start.is_none() {
                                if ch.is_whitespace() {
                                    ws_end = idx + ch.len_utf8();
                                } else if !ch.is_alphanumeric() && ch != '_' {
                                    // Found first punctuation character
                                    punct_start = Some(idx);
                                    punct_end = idx + ch.len_utf8();
                                    ws_end = punct_end;
                                } else {
                                    // Hit word character, stop
                                    break;
                                }
                            } else {
                                // We already found punctuation, continue scanning
                                if ch.is_whitespace() {
                                    ws_end = idx + ch.len_utf8();
                                } else if !ch.is_alphanumeric() && ch != '_' {
                                    // Found additional punctuation character
                                    punct_end = idx + ch.len_utf8();
                                    ws_end = punct_end;
                                } else {
                                    // Hit word character, stop
                                    break;
                                }
                            }
                        }

                        // Check if punctuation fits on current line
                        if punct_end > 0 {
                            let punct_width = UnicodeWidthStr::width(&text[..punct_end]);
                            if current_width + punct_width <= line_limit {
                                // Punctuation fits, add it to current line and trim trailing space
                                let punct_text = text[..punct_end].to_string();
                                current_line.push((Span::styled(punct_text, style), kind.clone()));
                                // Note: current_width update not needed - will be reset when wrapping
                                text = text[ws_end..].to_string();

                                if text.is_empty() {
                                    // After processing boundary characters, nothing remains
                                    wrapped_lines.push(std::mem::take(&mut current_line));
                                    current_width = 0;
                                    line_limit = continuation_width;
                                    continue;
                                }
                            } else if punct_start == Some(0) {
                                // Punctuation doesn't fit AND it's directly adjacent (no leading space).
                                // Before backtracking, check if we'd make progress:
                                // If styled span + punctuation won't fit on a new line, don't backtrack.
                                let punct_width = UnicodeWidthStr::width(&text[..punct_end]);
                                let last_span_width = current_line
                                    .last()
                                    .map(|(span, _)| UnicodeWidthStr::width(span.content.as_ref()))
                                    .unwrap_or(0);

                                // Only backtrack if styled word + punctuation will fit on continuation line
                                if last_span_width + punct_width <= continuation_width {
                                    // Backtrack: pop styled span, wrap current line, then add styled span
                                    // to next line followed by current text. This preserves the styling.
                                    if let Some((last_span, last_kind)) = current_line.pop() {
                                        // Wrap the current line (without the styled span)
                                        wrapped_lines.push(std::mem::take(&mut current_line));

                                        // Start new line with the styled span (preserving its style/kind)
                                        current_line.push((last_span, last_kind));
                                        current_width = UnicodeWidthStr::width(
                                            current_line[0].0.content.as_ref(),
                                        );
                                        line_limit = continuation_width;

                                        // Restart the while loop to reprocess current text on the new line
                                        continue;
                                    }
                                }
                                // else: backtracking won't help, let normal wrapping handle it
                            } else if punct_start.is_some() {
                                // Punctuation doesn't fit but "stands alone" (has leading space).
                                // Trim the leading space and let punctuation wrap normally.
                                text = text.trim_start().to_string();
                            }
                        } else if ws_end > 0 {
                            // Just whitespace, trim it
                            text = text[ws_end..].to_string();

                            if text.is_empty() {
                                // After trimming whitespace, nothing remains
                                wrapped_lines.push(std::mem::take(&mut current_line));
                                current_width = 0;
                                line_limit = continuation_width;
                                continue;
                            }
                        }
                    }

                    wrapped_lines.push(std::mem::take(&mut current_line));
                    current_width = 0;
                    line_limit = continuation_width;
                    continue;
                }
                let left = text[..break_pos].trim_end();
                if !left.is_empty() {
                    let left_width = UnicodeWidthStr::width(left);
                    if current_width > 0 && current_width + left_width > line_limit {
                        wrapped_lines.push(std::mem::take(&mut current_line));
                        current_width = 0;
                        line_limit = continuation_width;
                    }
                    if left_width > 0 {
                        current_line.push((Span::styled(left.to_string(), style), kind.clone()));
                        current_width += left_width;
                    }
                }
                text = text[break_pos..].trim_start().to_string();
                if !text.is_empty() {
                    wrapped_lines.push(std::mem::take(&mut current_line));
                    current_width = 0;
                    line_limit = continuation_width;
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
        let wrapped = wrap_spans_to_width_generic_shared(&spans, 6, 0);
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

    #[test]
    fn wrap_preserves_zwj_clusters() {
        let spans = vec![(Span::raw("👩‍💻👩‍💻"), SpanKind::Text)];
        let wrapped = wrap_spans_to_width_generic_shared(&spans, 2, 0);
        let lines: Vec<String> = wrapped
            .into_iter()
            .map(|line| {
                line.iter()
                    .map(|(s, _)| s.content.as_ref())
                    .collect::<String>()
            })
            .filter(|s| !s.is_empty())
            .collect();
        assert_eq!(lines, vec!["👩‍💻", "👩‍💻"]);
    }

    #[test]
    fn wrap_preserves_skin_tone_modifiers() {
        let spans = vec![(Span::raw("👍🏽👍🏽"), SpanKind::Text)];
        let wrapped = wrap_spans_to_width_generic_shared(&spans, 2, 0);
        let lines: Vec<String> = wrapped
            .into_iter()
            .map(|line| {
                line.iter()
                    .map(|(s, _)| s.content.as_ref())
                    .collect::<String>()
            })
            .filter(|s| !s.is_empty())
            .collect();
        assert_eq!(lines, vec!["👍🏽", "👍🏽"]);
    }
}
