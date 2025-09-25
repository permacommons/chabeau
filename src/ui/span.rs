/// Semantic classification for rendered spans. Enables downstream
/// consumers (scroll, selection, accessibility) to make decisions
/// without relying on styling heuristics such as underline detection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SpanKind {
    /// Default text content with no special interaction.
    Text,
    /// A user message prefix (e.g., "You: ") rendered ahead of content.
    UserPrefix,
    /// A hyperlink span emitted by the markdown renderer.
    Link,
}
