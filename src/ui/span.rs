//! Semantic span metadata for rendered content.
//!
//! This module defines [`SpanKind`] which classifies rendered text spans
//! for downstream consumers like scroll calculation, selection logic, and
//! accessibility features.
//!
//! Interactive elements (links, code blocks) carry rich metadata enabling
//! cached identification and efficient navigation without re-rendering.
//!
//! # Examples
//!
//! Creating and querying code block metadata:
//!
//! ```
//! use chabeau::ui::span::{SpanKind, CodeBlockMeta};
//!
//! // Create code block metadata
//! let kind = SpanKind::code_block(Some("rust"), 0);
//!
//! // Query metadata
//! if let Some(meta) = kind.code_block_meta() {
//!     assert_eq!(meta.language(), Some("rust"));
//!     assert_eq!(meta.block_index(), 0);
//! }
//! ```
//!
//! Extracting code blocks from rendered output:
//!
//! ```
//! use chabeau::ui::span::{extract_code_blocks, SpanKind};
//!
//! let metadata = vec![
//!     vec![SpanKind::Text],
//!     vec![SpanKind::code_block(Some("python"), 0)],
//!     vec![SpanKind::code_block(Some("python"), 0)],
//!     vec![SpanKind::Text],
//! ];
//!
//! let blocks = extract_code_blocks(&metadata);
//! assert_eq!(blocks.len(), 1);
//! assert_eq!(blocks[0].start_line, 1);
//! assert_eq!(blocks[0].end_line, 2);
//! ```

use std::sync::Arc;

/// Semantic classification for rendered spans. Enables downstream
/// consumers (scroll, selection, accessibility) to make decisions
/// without relying on styling heuristics such as underline detection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpanKind {
    /// Default text content with no special interaction.
    Text,
    /// A user message prefix (e.g., "You: ") rendered ahead of content.
    UserPrefix,
    /// An app message prefix rendered ahead of app-authored content.
    AppPrefix,
    /// A hyperlink span emitted by the markdown renderer.
    Link(LinkMeta),
    /// A code block span rendered from a fenced code block.
    CodeBlock(CodeBlockMeta),
}

impl SpanKind {
    #[inline]
    pub fn is_link(&self) -> bool {
        matches!(self, SpanKind::Link(_))
    }

    #[inline]
    pub fn link_meta(&self) -> Option<&LinkMeta> {
        match self {
            SpanKind::Link(meta) => Some(meta),
            _ => None,
        }
    }

    #[inline]
    pub fn is_user_prefix(&self) -> bool {
        matches!(self, SpanKind::UserPrefix)
    }

    #[inline]
    pub fn is_app_prefix(&self) -> bool {
        matches!(self, SpanKind::AppPrefix)
    }

    #[inline]
    pub fn is_prefix(&self) -> bool {
        self.is_user_prefix() || self.is_app_prefix()
    }

    #[inline]
    pub fn link(href: impl Into<String>) -> Self {
        SpanKind::Link(LinkMeta::new(href))
    }

    /// Returns true if this span is part of a code block.
    #[inline]
    pub fn is_code_block(&self) -> bool {
        matches!(self, SpanKind::CodeBlock(_))
    }

    /// Returns code block metadata if this span is a code block.
    #[inline]
    pub fn code_block_meta(&self) -> Option<&CodeBlockMeta> {
        match self {
            SpanKind::CodeBlock(meta) => Some(meta),
            _ => None,
        }
    }

    /// Creates a code block span kind with the given metadata.
    #[inline]
    pub fn code_block(language: Option<impl Into<String>>, block_index: usize) -> Self {
        SpanKind::CodeBlock(CodeBlockMeta::new(language, block_index))
    }
}

#[cfg(test)]
impl SpanKind {
    #[inline]
    pub fn is_text(&self) -> bool {
        matches!(self, SpanKind::Text)
    }

    #[inline]
    pub fn link_href(&self) -> Option<&str> {
        match self {
            SpanKind::Link(meta) => Some(meta.href()),
            _ => None,
        }
    }
}

/// Metadata for a code block span.
///
/// Identifies a span as part of a fenced code block, tracking its
/// language tag and position within the rendered output.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CodeBlockMeta {
    language: Option<Arc<str>>,
    block_index: usize,
}

impl CodeBlockMeta {
    /// Creates metadata for a code block span.
    ///
    /// # Arguments
    ///
    /// * `language` - Optional language tag from fence (e.g., "rust")
    /// * `block_index` - Zero-based position of block in message
    pub fn new(language: Option<impl Into<String>>, block_index: usize) -> Self {
        Self {
            language: language.map(|s| Arc::<str>::from(s.into())),
            block_index,
        }
    }

    /// Returns the language tag if specified in the code fence.
    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }

    /// Returns the zero-based index of this block within the message.
    pub fn block_index(&self) -> usize {
        self.block_index
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LinkMeta {
    href: Arc<str>,
}

impl LinkMeta {
    pub fn new(href: impl Into<String>) -> Self {
        Self {
            href: Arc::<str>::from(href.into()),
        }
    }

    pub fn href(&self) -> &str {
        &self.href
    }
}

/// Position of a code block in rendered output.
///
/// Tracks the line range and metadata for a code block, enabling
/// navigation, selection, and content extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeBlockPosition {
    /// Zero-based block index within the message.
    pub block_index: usize,
    /// First line of the block (inclusive).
    pub start_line: usize,
    /// Last line of the block (inclusive).
    pub end_line: usize,
    /// Language tag if specified in the code fence.
    pub language: Option<String>,
}

/// Extracts code block positions from span metadata.
///
/// Scans metadata to identify code blocks and their line ranges,
/// enabling navigation and selection without re-parsing markdown.
///
/// # Arguments
///
/// * `metadata` - Span metadata parallel to rendered lines
///
/// # Returns
///
/// Vector of code block positions sorted by block index.
///
/// # Example
///
/// ```
/// use chabeau::ui::span::{extract_code_blocks, SpanKind};
/// use ratatui::text::{Line, Span};
///
/// let lines = vec![
///     Line::from(vec![Span::raw("fn main() {")]),
///     Line::from(vec![Span::raw("}")]),
/// ];
/// let metadata = vec![
///     vec![SpanKind::code_block(Some("rust"), 0)],
///     vec![SpanKind::code_block(Some("rust"), 0)],
/// ];
///
/// let blocks = extract_code_blocks(&metadata);
/// assert_eq!(blocks.len(), 1);
/// assert_eq!(blocks[0].block_index, 0);
/// assert_eq!(blocks[0].start_line, 0);
/// assert_eq!(blocks[0].end_line, 1);
/// ```
pub fn extract_code_blocks(metadata: &[Vec<SpanKind>]) -> Vec<CodeBlockPosition> {
    use std::collections::HashMap;

    let mut blocks: HashMap<usize, CodeBlockPosition> = HashMap::new();

    for (line_idx, line_meta) in metadata.iter().enumerate() {
        for span_kind in line_meta {
            if let Some(meta) = span_kind.code_block_meta() {
                blocks
                    .entry(meta.block_index())
                    .or_insert_with(|| CodeBlockPosition {
                        block_index: meta.block_index(),
                        start_line: line_idx,
                        end_line: line_idx,
                        language: meta.language().map(String::from),
                    })
                    .end_line = line_idx;
            }
        }
    }

    let mut result: Vec<_> = blocks.into_values().collect();
    result.sort_by_key(|b| b.block_index);
    result
}

/// Extracts the text content of a specific code block.
///
/// Scans lines and metadata to collect all text content belonging
/// to the specified code block, preserving line breaks and blank lines.
///
/// # Arguments
///
/// * `lines` - Rendered lines from the layout
/// * `metadata` - Span metadata parallel to lines
/// * `block_index` - Zero-based index of the block to extract
///
/// # Returns
///
/// The block's content as a string, or None if block_index is invalid.
///
/// # Example
///
/// ```
/// use chabeau::ui::span::{extract_code_block_content, SpanKind};
/// use ratatui::text::{Line, Span};
///
/// let lines = vec![
///     Line::from(vec![Span::raw("fn main() {")]),
///     Line::from(vec![Span::raw("    println!(\"Hello\");")]),
///     Line::from(vec![Span::raw("}")]),
/// ];
/// let metadata = vec![
///     vec![SpanKind::code_block(Some("rust"), 0)],
///     vec![SpanKind::code_block(Some("rust"), 0)],
///     vec![SpanKind::code_block(Some("rust"), 0)],
/// ];
///
/// let content = extract_code_block_content(&lines, &metadata, 0).unwrap();
/// assert!(content.contains("fn main()"));
/// ```
pub fn extract_code_block_content(
    lines: &[ratatui::text::Line],
    metadata: &[Vec<SpanKind>],
    block_index: usize,
) -> Option<String> {
    let mut content = String::new();
    let mut found_any = false;

    for (line, line_meta) in lines.iter().zip(metadata.iter()) {
        let mut line_content = String::new();
        let mut line_belongs_to_block = false;

        for (span, kind) in line.spans.iter().zip(line_meta.iter()) {
            if let Some(meta) = kind.code_block_meta() {
                if meta.block_index() == block_index {
                    line_content.push_str(&span.content);
                    line_belongs_to_block = true;
                    found_any = true;
                }
            }
        }

        // Append the line if it belongs to the target block, even if empty
        if line_belongs_to_block {
            if !content.is_empty() {
                content.push('\n');
            }
            content.push_str(&line_content);
        }
    }

    if found_any {
        Some(content)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_block_meta_stores_language() {
        let meta = CodeBlockMeta::new(Some("rust"), 0);
        assert_eq!(meta.language(), Some("rust"));
        assert_eq!(meta.block_index(), 0);
    }

    #[test]
    fn code_block_meta_handles_no_language() {
        let meta = CodeBlockMeta::new(None::<String>, 1);
        assert_eq!(meta.language(), None);
        assert_eq!(meta.block_index(), 1);
    }

    #[test]
    fn span_kind_recognizes_code_blocks() {
        let span = SpanKind::code_block(Some("python"), 0);
        assert!(span.is_code_block());
        assert!(!span.is_link());

        let meta = span.code_block_meta().unwrap();
        assert_eq!(meta.language(), Some("python"));
        assert_eq!(meta.block_index(), 0);
    }

    #[test]
    fn text_spans_are_not_code_blocks() {
        let span = SpanKind::Text;
        assert!(!span.is_code_block());
        assert!(span.code_block_meta().is_none());
    }

    #[test]
    fn extract_code_blocks_finds_all_blocks() {
        let metadata = vec![
            vec![SpanKind::Text],                          // Line 0: not a code block
            vec![SpanKind::code_block(Some("rust"), 0)],   // Line 1: block 0
            vec![SpanKind::code_block(Some("rust"), 0)],   // Line 2: block 0
            vec![SpanKind::Text],                          // Line 3: not a code block
            vec![SpanKind::code_block(Some("python"), 1)], // Line 4: block 1
            vec![SpanKind::code_block(None::<String>, 2)], // Line 5: block 2
        ];

        let blocks = extract_code_blocks(&metadata);

        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].block_index, 0);
        assert_eq!(blocks[1].block_index, 1);
        assert_eq!(blocks[2].block_index, 2);
    }

    #[test]
    fn extract_code_blocks_computes_line_ranges() {
        let metadata = vec![
            vec![SpanKind::code_block(Some("rust"), 0)], // Line 0
            vec![SpanKind::code_block(Some("rust"), 0)], // Line 1
            vec![SpanKind::code_block(Some("rust"), 0)], // Line 2
        ];

        let blocks = extract_code_blocks(&metadata);

        assert_eq!(blocks.len(), 1);
        let block = &blocks[0];
        assert_eq!(block.start_line, 0);
        assert_eq!(block.end_line, 2);
        assert_eq!(block.language, Some("rust".to_string()));
    }

    #[test]
    fn extract_code_blocks_preserves_language() {
        let metadata = vec![
            vec![SpanKind::code_block(Some("javascript"), 0)],
            vec![SpanKind::code_block(None::<String>, 1)],
        ];

        let blocks = extract_code_blocks(&metadata);

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].language, Some("javascript".to_string()));
        assert_eq!(blocks[1].language, None);
    }

    #[test]
    fn extract_code_blocks_handles_empty_metadata() {
        let metadata: Vec<Vec<SpanKind>> = vec![];
        let blocks = extract_code_blocks(&metadata);
        assert_eq!(blocks.len(), 0);
    }

    #[test]
    fn extract_code_blocks_handles_no_code_blocks() {
        let metadata = vec![
            vec![SpanKind::Text],
            vec![SpanKind::link("https://example.com")],
            vec![SpanKind::UserPrefix],
        ];

        let blocks = extract_code_blocks(&metadata);
        assert_eq!(blocks.len(), 0);
    }

    #[test]
    fn extract_code_block_content_retrieves_code() {
        use ratatui::text::{Line, Span};

        let lines = vec![
            Line::from(vec![Span::raw("fn main() {")]),
            Line::from(vec![Span::raw("    println!(\"Hello\");")]),
            Line::from(vec![Span::raw("}")]),
        ];
        let metadata = vec![
            vec![SpanKind::code_block(Some("rust"), 0)],
            vec![SpanKind::code_block(Some("rust"), 0)],
            vec![SpanKind::code_block(Some("rust"), 0)],
        ];

        let content = extract_code_block_content(&lines, &metadata, 0).unwrap();
        assert!(content.contains("fn main()"));
        assert!(content.contains("println!"));
        assert!(content.contains("}"));
    }

    #[test]
    fn extract_content_preserves_line_breaks() {
        use ratatui::text::{Line, Span};

        let lines = vec![
            Line::from(vec![Span::raw("line1")]),
            Line::from(vec![Span::raw("line2")]),
            Line::from(vec![Span::raw("line3")]),
        ];
        let metadata = vec![
            vec![SpanKind::code_block(Some("txt"), 0)],
            vec![SpanKind::code_block(Some("txt"), 0)],
            vec![SpanKind::code_block(Some("txt"), 0)],
        ];

        let content = extract_code_block_content(&lines, &metadata, 0).unwrap();
        let line_count = content.lines().count();
        assert_eq!(line_count, 3, "Should preserve 3 lines");
        assert_eq!(content, "line1\nline2\nline3");
    }

    #[test]
    fn extract_content_returns_none_for_invalid_index() {
        use ratatui::text::{Line, Span};

        let lines = vec![Line::from(vec![Span::raw("fn main() {}")])];
        let metadata = vec![vec![SpanKind::code_block(Some("rust"), 0)]];

        let content = extract_code_block_content(&lines, &metadata, 999);
        assert!(content.is_none(), "Invalid index should return None");
    }

    #[test]
    fn extract_content_handles_multiple_spans_per_line() {
        use ratatui::text::{Line, Span};

        let lines = vec![Line::from(vec![
            Span::raw("fn "),
            Span::raw("main"),
            Span::raw("() {}"),
        ])];
        let metadata = vec![vec![
            SpanKind::code_block(Some("rust"), 0),
            SpanKind::code_block(Some("rust"), 0),
            SpanKind::code_block(Some("rust"), 0),
        ]];

        let content = extract_code_block_content(&lines, &metadata, 0).unwrap();
        assert_eq!(content, "fn main() {}");
    }

    #[test]
    fn extract_content_selects_correct_block() {
        use ratatui::text::{Line, Span};

        let lines = vec![
            Line::from(vec![Span::raw("block 0 line 1")]),
            Line::from(vec![Span::raw("block 0 line 2")]),
            Line::from(vec![Span::raw("block 1 line 1")]),
            Line::from(vec![Span::raw("block 1 line 2")]),
        ];
        let metadata = vec![
            vec![SpanKind::code_block(Some("txt"), 0)],
            vec![SpanKind::code_block(Some("txt"), 0)],
            vec![SpanKind::code_block(Some("txt"), 1)],
            vec![SpanKind::code_block(Some("txt"), 1)],
        ];

        let content0 = extract_code_block_content(&lines, &metadata, 0).unwrap();
        assert!(content0.contains("block 0"));
        assert!(!content0.contains("block 1"));

        let content1 = extract_code_block_content(&lines, &metadata, 1).unwrap();
        assert!(content1.contains("block 1"));
        assert!(!content1.contains("block 0"));
    }

    #[test]
    fn extract_content_preserves_blank_lines() {
        use ratatui::text::{Line, Span};

        // Code block with blank lines
        let lines = vec![
            Line::from(vec![Span::raw("first line")]),
            Line::from(vec![Span::raw("")]), // blank line
            Line::from(vec![Span::raw("third line")]),
            Line::from(vec![Span::raw("")]), // another blank line
            Line::from(vec![Span::raw("fifth line")]),
        ];
        let metadata = vec![
            vec![SpanKind::code_block(Some("txt"), 0)],
            vec![SpanKind::code_block(Some("txt"), 0)],
            vec![SpanKind::code_block(Some("txt"), 0)],
            vec![SpanKind::code_block(Some("txt"), 0)],
            vec![SpanKind::code_block(Some("txt"), 0)],
        ];

        let content = extract_code_block_content(&lines, &metadata, 0).unwrap();

        // Should have exactly 5 lines (including 2 blank ones)
        let line_count = content.lines().count();
        assert_eq!(
            line_count, 5,
            "Should preserve all 5 lines including blank ones, got {} lines: {:?}",
            line_count, content
        );

        // Verify the exact structure
        assert_eq!(content, "first line\n\nthird line\n\nfifth line");
    }

    #[test]
    fn extract_content_omits_list_indent() {
        use crate::core::message::{Message, TranscriptRole};
        use crate::ui::markdown::{render_message_with_config, MessageRenderConfig};
        use crate::ui::theme::Theme;

        // Create a message with code in a list
        let msg = Message {
            role: TranscriptRole::Assistant,
            content: "1. Step one\n\n   ```python\n   def foo():\n       pass\n   ```\n"
                .to_string(),
        };

        let theme = Theme::dark_default();
        let config = MessageRenderConfig::markdown(true, false)
            .with_span_metadata()
            .with_terminal_width(Some(80), crate::ui::layout::TableOverflowPolicy::WrapCells);

        let rendered = render_message_with_config(&msg, &theme, config);
        let metadata = rendered.span_metadata.unwrap();

        // Extract the code block
        let content = extract_code_block_content(&rendered.lines, &metadata, 0)
            .expect("Should extract code block");

        // CRITICAL: Content should NOT include the 3-space list indent
        // The original code is column-zero:
        //   def foo():
        //       pass
        // NOT:
        //      def foo():
        //          pass
        assert!(
            content.starts_with("def foo():"),
            "Extracted code should start at column 0, not indented. Got: {:?}",
            content
        );

        assert!(
            !content.starts_with("   def"),
            "Extracted code must NOT include list indent padding. Got: {:?}",
            content
        );

        // Verify the content is exactly the original code
        assert_eq!(content, "def foo():\n    pass");
    }
}
