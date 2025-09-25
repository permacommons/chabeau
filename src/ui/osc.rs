#![cfg_attr(not(test), allow(dead_code))]

use std::borrow::Cow;

use crate::ui::span::SpanKind;
use ratatui::text::Line;

const OSC_PREFIX: &str = "\x1b]8;;";
const ST: &str = "\x1b\\";
const OSC_SUFFIX: &str = "\x1b]8;;\x1b\\";

/// Encapsulates the pieces required to write an OSC 8 hyperlink without
/// exposing partially constructed escape sequences.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OscHyperlink<'a> {
    prefix: String,
    text: Cow<'a, str>,
    suffix: &'static str,
}

impl<'a> OscHyperlink<'a> {
    /// Append the OSC hyperlink to the provided buffer. The method guarantees
    /// that the start and end escape sequences are written atomically.
    pub fn push_to(&self, buf: &mut String) {
        buf.push_str(&self.prefix);
        buf.push_str(self.text.as_ref());
        buf.push_str(self.suffix);
    }

    /// Returns the OSC-encoded string in a single allocation for convenience
    /// in tests and log output.
    pub fn as_encoded_string(&self) -> String {
        let mut out =
            String::with_capacity(self.prefix.len() + self.text.len() + self.suffix.len());
        self.push_to(&mut out);
        out
    }
}

/// Attempt to encode `text` as an OSC 8 hyperlink pointing to `href`. Returns
/// `None` when emission would be unsafe (empty segments or control characters)
/// so callers can gracefully fall back to plain text.
pub fn encode_hyperlink<'a>(text: &'a str, href: &str) -> Option<OscHyperlink<'a>> {
    if text.is_empty() || href.is_empty() {
        return None;
    }

    if contains_disallowed_control(text) || contains_disallowed_control(href) {
        return None;
    }

    let mut prefix = String::with_capacity(OSC_PREFIX.len() + href.len() + ST.len());
    prefix.push_str(OSC_PREFIX);
    prefix.push_str(href);
    prefix.push_str(ST);

    Some(OscHyperlink {
        prefix,
        text: Cow::Borrowed(text),
        suffix: OSC_SUFFIX,
    })
}

fn contains_disallowed_control(input: &str) -> bool {
    input.bytes().any(|b| (b < 0x20 && b != b'\t') || b == 0x1b)
}

pub fn encode_line_with_links(line: &Line, kinds: &[SpanKind]) -> String {
    let mut out = String::new();
    for (span, kind) in line.spans.iter().zip(kinds.iter()) {
        let content = span.content.as_ref();
        if let Some(meta) = kind.link_meta() {
            if let Some(link) = encode_hyperlink(content, meta.href()) {
                link.push_to(&mut out);
                continue;
            }
        }
        out.push_str(content);
    }
    out
}

pub fn encode_lines_with_links(lines: &[Line], metadata: &[Vec<SpanKind>]) -> Vec<String> {
    lines
        .iter()
        .zip(metadata.iter())
        .map(|(line, kinds)| encode_line_with_links(line, kinds))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{contains_disallowed_control, encode_hyperlink, OSC_PREFIX, OSC_SUFFIX, ST};
    use crate::ui::span::SpanKind;
    use ratatui::text::{Line, Span};

    #[test]
    fn encode_hyperlink_wraps_text_with_balanced_sequences() {
        let link = encode_hyperlink("Rust", "https://www.rust-lang.org").expect("link");
        let encoded = link.as_encoded_string();
        assert!(encoded.starts_with(OSC_PREFIX));
        assert!(encoded.ends_with(OSC_SUFFIX));
        assert_eq!(encoded.matches(OSC_PREFIX).count(), 2);
        assert_eq!(encoded.matches(OSC_SUFFIX).count(), 1);
        assert_eq!(encoded.matches(ST).count(), 2);
        assert!(encoded.contains("Rust"));
    }

    #[test]
    fn encode_hyperlink_rejects_empty_segments() {
        assert!(encode_hyperlink("", "https://example.com").is_none());
        assert!(encode_hyperlink("Example", "").is_none());
    }

    #[test]
    fn encode_hyperlink_rejects_control_bytes() {
        assert!(encode_hyperlink("hi", "bad\u{1b}url").is_none());
        assert!(encode_hyperlink("bad\u{7}text", "https://example.com").is_none());
    }

    #[test]
    fn control_detection_catches_bel_and_escape() {
        assert!(contains_disallowed_control("\u{1b}"));
        assert!(contains_disallowed_control("\u{7}"));
        assert!(contains_disallowed_control("line\n"));
        assert!(contains_disallowed_control("carriage\rreturn"));
        assert!(!contains_disallowed_control("tab\tallowed"));
        assert!(!contains_disallowed_control("normal"));
    }

    #[test]
    fn encode_line_with_links_applies_osc_sequences() {
        let line = Line::from(vec![Span::raw("Visit "), Span::raw("Rust"), Span::raw("!")]);
        let kinds = vec![
            SpanKind::Text,
            SpanKind::link("https://www.rust-lang.org"),
            SpanKind::Text,
        ];
        let encoded = super::encode_line_with_links(&line, &kinds);
        assert!(encoded.contains("Visit "));
        assert!(encoded.contains("Rust"));
        assert!(encoded.ends_with(format!("{OSC_SUFFIX}!").as_str()));
        assert_eq!(encoded.matches(OSC_PREFIX).count(), 2);
        assert_eq!(encoded.matches(OSC_SUFFIX).count(), 1);
    }

    #[test]
    fn encode_lines_with_links_handles_adjacent_links() {
        let lines = vec![Line::from(vec![Span::raw("Rust"), Span::raw("Go")])];
        let kinds = vec![vec![
            SpanKind::link("https://www.rust-lang.org"),
            SpanKind::link("https://go.dev"),
        ]];
        let encoded = super::encode_lines_with_links(&lines, &kinds);
        assert_eq!(encoded.len(), 1);
        let first = &encoded[0];
        assert!(first.contains("Rust"));
        assert!(first.contains("Go"));
        assert_eq!(first.matches(OSC_PREFIX).count(), 4);
        assert_eq!(first.matches(OSC_SUFFIX).count(), 2);
    }
}
