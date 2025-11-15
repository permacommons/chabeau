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
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_text(&self) -> bool {
        matches!(self, SpanKind::Text)
    }

    #[inline]
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn link_href(&self) -> Option<&str> {
        match self {
            SpanKind::Link(meta) => Some(meta.href()),
            _ => None,
        }
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

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn href(&self) -> &str {
        &self.href
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
}
