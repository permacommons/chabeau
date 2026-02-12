use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::collections::HashSet;

/// Check if there's a blank line immediately before the given position in the content.
fn has_blank_line_before(content: &str, pos: usize) -> bool {
    let line_start = content[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    if line_start <= 1 {
        return false;
    }

    let before_newline = line_start - 1;
    let prev_content = &content[..before_newline];

    if let Some(prev_line_start) = prev_content.rfind('\n') {
        let prev_line = &prev_content[prev_line_start + 1..];
        prev_line.trim().is_empty()
    } else {
        prev_content.trim().is_empty()
    }
}

/// Find list item indices (0-based) that should have blank lines before them.
pub(super) fn find_items_needing_blank_lines(content: &str) -> HashSet<usize> {
    let mut result = HashSet::new();
    let parser = Parser::new_ext(content, Options::all()).into_offset_iter();
    let mut item_index = 0;
    let mut list_item_counts: Vec<usize> = Vec::new();

    for (event, range) in parser {
        match event {
            Event::Start(Tag::List(_)) => list_item_counts.push(0),
            Event::End(TagEnd::List(_)) => {
                list_item_counts.pop();
            }
            Event::Start(Tag::Item) => {
                let depth = list_item_counts.len();
                let in_list_index = list_item_counts.last_mut().map(|count| {
                    let current = *count;
                    *count += 1;
                    current
                });

                if let Some(in_list_index) = in_list_index {
                    let should_check_blank = in_list_index > 0 || depth > 1;
                    if should_check_blank && has_blank_line_before(content, range.start) {
                        result.insert(item_index);
                    }
                }

                item_index += 1;
            }
            _ => {}
        }
    }

    result
}
