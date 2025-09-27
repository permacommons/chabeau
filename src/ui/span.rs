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
    /// A hyperlink span emitted by the markdown renderer.
    Link(LinkMeta),
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
