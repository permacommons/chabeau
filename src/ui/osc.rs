#[cfg(test)]
mod tests {
    use super::{
        contains_disallowed_control, encode_hyperlink, encode_line_with_links,
        encode_lines_with_links, OSC_PREFIX, OSC_SUFFIX,
    };
    use crate::ui::span::SpanKind;
    use ratatui::text::{Line, Span};

    #[test]
    fn encode_hyperlink_wraps_text_with_balanced_sequences() {
        let link = encode_hyperlink("Rust", "https://www.rust-lang.org").expect("link");
        let encoded = link.as_encoded_string();
        assert!(encoded.ends_with(OSC_SUFFIX));
        assert_eq!(encoded.matches(OSC_SUFFIX).count(), 1);
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
    fn encode_lines_with_links_wraps_each_link_segment() {
        let lines = vec![Line::from(vec![
            Span::raw("Intro "),
            Span::raw("Rust"),
            Span::raw(" and "),
            Span::raw("Go"),
        ])];
        let metadata = vec![vec![
            SpanKind::Text,
            SpanKind::link("https://www.rust-lang.org"),
            SpanKind::Text,
            SpanKind::link("https://go.dev"),
        ]];

        let encoded = encode_lines_with_links(&lines, &metadata);
        assert_eq!(encoded.len(), 1);
        let line = &encoded[0];
        assert!(line.contains("Intro"));
        assert!(line.contains("Rust"));
        assert!(line.contains("Go"));
        assert!(line.matches(OSC_PREFIX).count() >= 2);
        assert!(line.matches(OSC_SUFFIX).count() >= 2);
    }

    #[test]
    fn encode_line_with_links_returns_plain_text_without_metadata() {
        let line = Line::from(vec![Span::raw("Rust")]);
        let encoded = encode_line_with_links(&line, None);
        assert_eq!(encoded, "Rust");
    }
}

#[cfg(test)]
use std::borrow::Cow;
#[cfg(test)]
use crate::ui::span::SpanKind;
#[cfg(test)]
use ratatui::text::Line;

#[cfg(test)]
const OSC_PREFIX: &str = "\x1b]8;;";
#[cfg(test)]
const ST: &str = "\x1b\\";
#[cfg(test)]
const OSC_SUFFIX: &str = "\x1b]8;;\x1b\\";

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OscHyperlink<'a> {
    prefix: String,
    text: Cow<'a, str>,
    suffix: &'static str,
}

#[cfg(test)]
impl<'a> OscHyperlink<'a> {
    pub fn push_to(&self, buf: &mut String) {
        buf.push_str(&self.prefix);
        buf.push_str(self.text.as_ref());
        buf.push_str(self.suffix);
    }

    pub fn as_encoded_string(&self) -> String {
        let mut out =
            String::with_capacity(self.prefix.len() + self.text.len() + self.suffix.len());
        self.push_to(&mut out);
        out
    }
}

#[cfg(test)]
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

#[cfg(test)]
pub fn encode_line_with_links(line: &Line<'_>, kinds: Option<&[SpanKind]>) -> String {
    let mut encoded = String::new();

    for (idx, span) in line.spans.iter().enumerate() {
        let text = span.content.as_ref();
        let maybe_link = kinds
            .and_then(|meta| meta.get(idx))
            .and_then(|kind| kind.link_meta())
            .and_then(|meta| encode_hyperlink(text, meta.href()));

        if let Some(link) = maybe_link {
            encoded.push_str(&link.as_encoded_string());
        } else {
            encoded.push_str(text);
        }
    }

    encoded
}

#[cfg(test)]
pub fn encode_lines_with_links(lines: &[Line<'static>], metadata: &[Vec<SpanKind>]) -> Vec<String> {
    lines
        .iter()
        .enumerate()
        .map(|(idx, line)| {
            let kinds = metadata.get(idx).map(|vec| vec.as_slice());
            encode_line_with_links(line, kinds)
        })
        .collect()
}

#[cfg(test)]
fn contains_disallowed_control(input: &str) -> bool {
    input
        .bytes()
        .any(|b| (b < 0x20 && b != b'\t') || b == b'\x1b')
}
