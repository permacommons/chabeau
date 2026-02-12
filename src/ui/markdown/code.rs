use crate::ui::span::SpanKind;
use crate::ui::theme::Theme;
use pulldown_cmark::CodeBlockKind;
use ratatui::text::{Line, Span};

pub(super) fn language_hint_from_codeblock_kind(kind: CodeBlockKind) -> String {
    match kind {
        CodeBlockKind::Indented => String::new(),
        CodeBlockKind::Fenced(info) => info.split_ascii_whitespace().next().unwrap_or("").into(),
    }
}

pub(super) fn push_codeblock_text(code_block_lines: &mut Vec<String>, text: &str) {
    for l in text.lines() {
        code_block_lines.push(detab(l));
    }
}

fn plain_codeblock_lines(code_block_lines: &[String], theme: &Theme) -> Vec<Line<'static>> {
    let mut style = theme.md_codeblock_text_style();
    if let Some(bg) = theme.md_codeblock_bg_color() {
        style = style.bg(bg);
    }
    code_block_lines
        .iter()
        .map(|line| Line::from(vec![Span::styled(line.clone(), style)]))
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn flush_code_block_buffer(
    code_block_lines: &mut Vec<String>,
    syntax_enabled: bool,
    language_hint: Option<&str>,
    theme: &Theme,
    lines: &mut Vec<Line<'static>>,
    span_metadata: Option<&mut Vec<Vec<SpanKind>>>,
    list_indent: usize,
    block_index: usize,
) {
    if code_block_lines.is_empty() {
        return;
    }

    let produced_lines = if syntax_enabled {
        let joined = code_block_lines.join("\n");
        crate::utils::syntax::highlight_code_block(language_hint.unwrap_or(""), &joined, theme)
            .unwrap_or_else(|| plain_codeblock_lines(code_block_lines, theme))
    } else {
        plain_codeblock_lines(code_block_lines, theme)
    };

    let indent = (list_indent > 0).then(|| " ".repeat(list_indent));

    if let Some(metadata) = span_metadata {
        for mut line in produced_lines {
            let has_indent = if let Some(indent) = indent.as_ref() {
                line.spans.insert(0, Span::raw(indent.clone()));
                true
            } else {
                false
            };

            let lang = language_hint.and_then(|s| if s.is_empty() { None } else { Some(s) });
            let code_block_kind = SpanKind::code_block(lang, block_index);

            let mut line_metadata = Vec::with_capacity(line.spans.len());
            for (i, _) in line.spans.iter().enumerate() {
                if i == 0 && has_indent {
                    line_metadata.push(SpanKind::Text);
                } else {
                    line_metadata.push(code_block_kind.clone());
                }
            }

            metadata.push(line_metadata);
            lines.push(line);
        }
    } else {
        for mut line in produced_lines {
            if let Some(indent) = indent.as_ref() {
                line.spans.insert(0, Span::raw(indent.clone()));
            }
            lines.push(line);
        }
    }

    code_block_lines.clear();
}

fn detab(s: &str) -> String {
    s.replace('\t', "    ")
}
