use ratatui::text::{Line, Span};

/// Injects OSC 8 hyperlinks into a vector of `Line`s by replacing markers.
pub fn arm_links(lines: &[Line<'_>], urls: &[String]) -> Vec<Line<'static>> {
    if urls.is_empty() {
        return lines.iter().map(clone_line_to_static).collect();
    }

    let mut new_lines = Vec::new();
    let mut url_it = urls.iter();

    for line in lines {
        let mut new_spans: Vec<Span> = Vec::new();
        for span in &line.spans {
            let mut last_pos = 0;
            for (i, marker) in span.content.match_indices(|c| c == '\u{E000}' || c == '\u{E001}') {
                if i > last_pos {
                    new_spans.push(Span::styled(
                        span.content[last_pos..i].to_string(),
                        span.style,
                    ));
                }

                if marker == "\u{E000}" {
                    if let Some(url) = url_it.next() {
                        new_spans.push(Span::raw(format!("\x1B]8;;{}\x1B\\", url)));
                    }
                } else {
                    new_spans.push(Span::raw("\x1B]8;;\x1B\\"));
                }
                last_pos = i + marker.len();
            }

            if last_pos < span.content.len() {
                new_spans.push(Span::styled(
                    span.content[last_pos..].to_string(),
                    span.style,
                ));
            }
        }
        new_lines.push(Line::from(new_spans));
    }

    new_lines
}

fn clone_line_to_static(line: &Line<'_>) -> Line<'static> {
    Line::from(
        line.spans
            .iter()
            .map(|s| Span::styled(s.content.to_string(), s.style))
            .collect::<Vec<_>>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Style, Modifier};
    use ratatui::text::{Line, Span};

    #[test]
    fn test_arm_simple_link() {
        let lines = vec![Line::from(vec![Span::raw("Here is a "), Span::raw("\u{E000}link\u{E001}"), Span::raw(" to test.")])];
        let urls = vec!["http://example.com".to_string()];
        let new_lines = arm_links(&lines, &urls);

        let result: String = new_lines.iter().map(|l| l.to_string()).collect();
        let expected = "Here is a \x1B]8;;http://example.com\x1B\\link\x1B]8;;\x1B\\ to test.";
        assert_eq!(result, expected);
    }

    #[test]
    fn test_arm_link_with_styled_text() {
        let lines = vec![Line::from(vec![
            Span::raw("A link with "),
            Span::styled("\u{E000}bold\u{E001}", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" text."),
        ])];
        let urls = vec!["http://example.com".to_string()];
        let new_lines = arm_links(&lines, &urls);

        assert_eq!(new_lines.len(), 1);

        let mut combined_text = String::new();
        for span in &new_lines[0].spans {
            combined_text.push_str(&span.content);
        }

        let expected_text = "A link with \x1B]8;;http://example.com\x1B\\bold\x1B]8;;\x1B\\ text.";
        assert_eq!(combined_text, expected_text);
    }

    #[test]
    fn test_arm_link_split_across_spans() {
        let lines = vec![Line::from(vec![
            Span::raw("A link with "),
            Span::raw("\u{E000}"),
            Span::styled("bold", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" and "),
            Span::styled("italic", Style::default().add_modifier(Modifier::ITALIC)),
            Span::raw("\u{E001}"),
            Span::raw(" text."),
        ])];
        let urls = vec!["http://example.com".to_string()];
        let new_lines = arm_links(&lines, &urls);

        assert_eq!(new_lines.len(), 1);
        let result: String = new_lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        let expected = "A link with \x1B]8;;http://example.com\x1B\\bold and italic\x1B]8;;\x1B\\ text.";
        assert_eq!(result, expected);
    }

    #[test]
    fn test_arm_multiple_links() {
        let lines = vec![Line::from(vec![
            Span::raw("Here is "),
            Span::raw("\u{E000}link1\u{E001}"),
            Span::raw(" and "),
            Span::raw("\u{E000}link2\u{E001}"),
            Span::raw("."),
        ])];
        let urls = vec![
            "http://example.com/1".to_string(),
            "http://example.com/2".to_string(),
        ];
        let new_lines = arm_links(&lines, &urls);

        assert_eq!(new_lines.len(), 1);
        let result: String = new_lines.iter().map(|l| l.to_string()).collect();
        let expected = "Here is \x1B]8;;http://example.com/1\x1B\\link1\x1B]8;;\x1B\\ and \x1B]8;;http://example.com/2\x1B\\link2\x1B]8;;\x1B\\.";
        assert_eq!(result, expected);
    }
}
