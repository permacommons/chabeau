# Span Metadata Extension for Code Blocks

**Created**: 2025-11-15
**Status**: ✅ Complete
**Effort Estimate**: L (sequential phases with test-first approach)

> **T-Shirt Sizing**: XS = Tiny task, S = Small task, M = Medium task, L = Large task, XL = Extra large task

## Implementation Complete

All phases completed successfully. See [Post-Implementation Notes](#post-implementation-notes) for details on bugs discovered and fixed during rollout.

## Executive Summary

Code blocks in the TUI currently lack semantic metadata in the span rendering system. This requires special-case logic for block selection that recomputes code block positions on every navigation keypress, preventing cache usage and creating architectural inconsistency with how links are handled.

**Goal**: Extend `SpanKind` to include code block metadata, enabling cached code block identification, consistent interactive element architecture, and simplified block selection logic.

**Benefits**:
- **Performance**: Eliminate redundant markdown re-rendering during block navigation
- **Consistency**: Unify interactive elements (links and code blocks) under the span metadata system
- **Simplicity**: Replace 100+ lines of special-case code with cached metadata queries
- **Extensibility**: Unlock future TUI interactions (hover previews, inline expansion, etc.)

---

## Current State Analysis

### Architecture Overview

The span metadata system (`src/ui/span.rs`) classifies rendered content for downstream consumers:

```rust
pub enum SpanKind {
    Text,
    UserPrefix,
    AppPrefix,
    Link(LinkMeta),  // ← Interactive element with metadata
}
```

**Links benefit from span metadata**:
- Identified during initial render and cached
- Selection logic queries cached metadata (O(1))
- No re-rendering needed for interaction

**Code blocks lack span metadata**:
- Not tracked in `SpanKind` enum
- Require full re-render to identify positions
- Special-case logic in `compute_codeblock_ranges_with_width_and_policy()`
- Cache invalidated during block selection mode

### Performance Impact

**Current block selection flow** (`src/ui/chat_loop/modes.rs:286-293`):
```rust
// Every Up/Down keypress in block select mode
let ranges = compute_codeblock_ranges_with_width_and_policy(
    &app.ui.messages,      // Re-render all messages
    &app.ui.theme,
    Some(term_width),
    policy,
    syntax_enabled,
    Some(&user_display_name),
);
```

**Rendering impact** (`src/ui/renderer.rs:65-77`):
```rust
// Cannot use cached prewrapped lines in block select mode
if app.ui.in_block_select_mode() {
    // Full re-render with highlighting
    let layout = build_layout_with_codeblock_highlight_and_flags_and_width(...);
}
```

### Code Locations Requiring Changes

**Core span system**:
- `src/ui/span.rs` - Add `CodeBlock` variant with metadata
- `src/ui/markdown.rs` - Emit code block metadata during rendering

**Code block identification**:
- `src/ui/markdown.rs:775-900` - `compute_codeblock_ranges()` functions
- `src/ui/layout.rs` - Layout computation with code block ranges

**Block selection logic**:
- `src/ui/chat_loop/modes.rs:275-388` - Block selection event handling
- `src/ui/chat_loop/keybindings/handlers.rs:498-551` - Ctrl+B handler
- `src/ui/renderer.rs:45-80` - Block selection rendering

**Scrolling and navigation**:
- `src/utils/scroll.rs` - Code block highlighting in layouts

---

## Implementation Phases

### Phase 0: Test Infrastructure (Size: S)

**Goal**: Establish test fixtures and edge case definitions before implementation

#### Task 0.1: Define Test Fixtures (Size: XS)

Create `src/ui/markdown/codeblock_fixtures.rs` with canonical test cases:

```rust
/// Test fixture: single code block in assistant message
pub fn single_block() -> Message {
    Message {
        role: ROLE_ASSISTANT.to_string(),
        content: "Here's a function:\n\n```rust\nfn main() {}\n```\n".to_string(),
    }
}

/// Test fixture: multiple code blocks with different languages
pub fn multiple_blocks() -> Message { /* ... */ }

/// Test fixture: code block in ordered list with indentation
pub fn nested_in_list() -> Message { /* ... */ }

/// Test fixture: code block with long lines requiring wrapping
pub fn wrapped_code() -> Message { /* ... */ }

/// Test fixture: empty code block (edge case)
pub fn empty_block() -> Message { /* ... */ }

/// Test fixture: code block immediately adjacent to text (no newlines)
pub fn adjacent_to_text() -> Message { /* ... */ }
```

**Edge cases to cover**:
1. ✅ Single code block
2. ✅ Multiple code blocks in one message
3. ✅ Code blocks across multiple messages
4. ✅ Nested in lists (indented)
5. ✅ Wrapped code (long lines)
6. ✅ Empty code blocks
7. ✅ Adjacent to text (no surrounding newlines)
8. ✅ Code blocks in user messages vs assistant messages
9. ✅ Code blocks with various language tags (rust, python, bash, txt, none)
10. ✅ Tables containing code blocks (if supported)

**Deliverable**: Test module with 10 fixtures covering edge cases

#### Task 0.2: Write Failing Tests for Span Metadata (Size: XS)

Add tests in `src/ui/markdown/tests.rs`:

```rust
#[test]
fn code_block_spans_have_metadata() {
    let msg = fixtures::single_block();
    let theme = Theme::dark_default();
    let cfg = MessageRenderConfig::markdown(true, false);

    let rendered = render_message_with_config(&msg, &theme, cfg);
    let metadata = rendered.span_metadata;

    // Find spans that should be code blocks
    let code_spans: Vec<_> = metadata.iter()
        .flat_map(|line| line.iter())
        .filter(|kind| matches!(kind, SpanKind::CodeBlock(_)))
        .collect();

    assert!(!code_spans.is_empty(), "Code block should have CodeBlock metadata");

    // Verify metadata contains language and block index
    if let SpanKind::CodeBlock(meta) = &code_spans[0] {
        assert_eq!(meta.language(), Some("rust"));
        assert_eq!(meta.block_index(), 0);
    }
}

#[test]
fn multiple_code_blocks_have_unique_indices() { /* ... */ }

#[test]
fn empty_code_block_has_metadata() { /* ... */ }

#[test]
fn wrapped_code_preserves_metadata_across_lines() { /* ... */ }
```

**Deliverable**: 8-10 failing tests for code block span metadata

#### Task 0.3: Write Tests for Cache Behavior (Size: XS)

Add tests in `src/core/app/tests.rs`:

```rust
#[test]
fn block_selection_uses_cached_metadata() {
    let mut app = create_test_app();
    app.ui.messages.push_back(fixtures::multiple_blocks());

    // First render caches metadata
    let width = 80u16;
    let _lines = app.get_prewrapped_lines_cached(width);
    let metadata = app.get_prewrapped_span_metadata_cached(width);

    // Count code block spans in cache
    let cached_blocks = count_code_blocks_in_metadata(metadata);
    assert_eq!(cached_blocks, 3, "Should cache 3 code blocks");

    // Enter block select mode
    app.ui.enter_block_select_mode(0);

    // Navigation should not invalidate cache
    let metadata_after = app.get_prewrapped_span_metadata_cached(width);
    assert!(Arc::ptr_eq(metadata, metadata_after),
            "Block navigation should reuse cached metadata");
}

#[test]
fn cache_invalidates_on_message_change() { /* ... */ }

#[test]
fn cache_invalidates_on_width_change() { /* ... */ }
```

**Deliverable**: 3-5 failing tests for cache behavior

**Phase 0 Success Criteria**:
- ✅ All tests compile but fail (expected)
- ✅ Edge cases comprehensively covered
- ✅ Test fixtures are reusable across test modules
- ✅ `cargo test` runs without panics (failures expected)

---

### Phase 1: Core Span Metadata Extension (Size: M)

**Goal**: Extend `SpanKind` to support code blocks with metadata

#### Task 1.1: Define `CodeBlockMeta` Type (Size: XS)

Add to `src/ui/span.rs`:

```rust
/// Metadata for a code block span.
///
/// Identifies a span as part of a fenced code block, tracking its
/// language tag and position within the rendered output.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CodeBlockMeta {
    /// Language tag from the code fence (e.g., "rust", "python").
    /// None if no language was specified.
    language: Option<Arc<str>>,

    /// Zero-based index of this code block in the rendered message.
    /// Used for navigation and selection in block select mode.
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
```

**Tests**:
```rust
#[test]
fn code_block_meta_stores_language() {
    let meta = CodeBlockMeta::new(Some("rust"), 0);
    assert_eq!(meta.language(), Some("rust"));
}

#[test]
fn code_block_meta_handles_no_language() {
    let meta = CodeBlockMeta::new(None::<String>, 1);
    assert_eq!(meta.language(), None);
    assert_eq!(meta.block_index(), 1);
}
```

**Deliverable**: `CodeBlockMeta` type with 2 passing tests

#### Task 1.2: Add `CodeBlock` Variant to `SpanKind` (Size: XS)

Extend `src/ui/span.rs`:

```rust
pub enum SpanKind {
    Text,
    UserPrefix,
    AppPrefix,
    Link(LinkMeta),
    /// A code block span rendered from a fenced code block.
    CodeBlock(CodeBlockMeta),
}

impl SpanKind {
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
```

**Tests**:
```rust
#[test]
fn span_kind_recognizes_code_blocks() {
    let span = SpanKind::code_block(Some("python"), 0);
    assert!(span.is_code_block());
    assert!(!span.is_link());

    let meta = span.code_block_meta().unwrap();
    assert_eq!(meta.language(), Some("python"));
}

#[test]
fn text_spans_are_not_code_blocks() {
    let span = SpanKind::Text;
    assert!(!span.is_code_block());
    assert!(span.code_block_meta().is_none());
}
```

**Deliverable**: Extended `SpanKind` with 2 passing tests

#### Task 1.3: Update Markdown Renderer to Emit Code Block Metadata (Size: M)

Modify `src/ui/markdown.rs` to track code blocks during rendering:

```rust
struct MarkdownRenderer<'a> {
    // ... existing fields ...

    /// Tracks the current code block being rendered.
    /// Contains (language, block_index) when inside a code block.
    current_code_block: Option<(Option<String>, usize)>,

    /// Count of code blocks encountered in this message.
    code_block_count: usize,
}

impl<'a> MarkdownRenderer<'a> {
    // In start_event() handling Event::Start(Tag::CodeBlock(...)):
    fn enter_code_block(&mut self, kind: CodeBlockKind) {
        let language = match kind {
            CodeBlockKind::Fenced(lang) => {
                if lang.is_empty() {
                    None
                } else {
                    Some(lang.to_string())
                }
            }
            CodeBlockKind::Indented => None,
        };

        self.current_code_block = Some((language, self.code_block_count));
        self.code_block_count += 1;
    }

    // In push_span():
    fn push_span(&mut self, text: String, style: Style) {
        let kind = if let Some((ref lang, block_idx)) = self.current_code_block {
            SpanKind::code_block(lang.clone(), block_idx)
        } else if /* existing link logic */ {
            SpanKind::Link(link_meta)
        } else if /* existing prefix logic */ {
            SpanKind::UserPrefix
        } else {
            SpanKind::Text
        };

        // Push span and metadata
        self.current_line.push(Span::styled(text, style));
        self.current_metadata.push(kind);
    }
}
```

**Tests**:
```rust
#[test]
fn renderer_emits_code_block_metadata_for_fenced_blocks() {
    let content = "```rust\nfn main() {}\n```";
    let msg = Message {
        role: ROLE_ASSISTANT.to_string(),
        content: content.to_string(),
    };

    let rendered = render_message(&msg, &Theme::dark_default(), true, false);

    // Find all code block spans
    let code_spans: Vec<_> = rendered.span_metadata.iter()
        .flat_map(|line| line.iter())
        .filter(|k| k.is_code_block())
        .collect();

    assert!(!code_spans.is_empty(), "Should emit code block metadata");

    let meta = code_spans[0].code_block_meta().unwrap();
    assert_eq!(meta.language(), Some("rust"));
    assert_eq!(meta.block_index(), 0);
}

#[test]
fn multiple_blocks_have_sequential_indices() {
    let content = "```rust\ncode1\n```\n\nText\n\n```python\ncode2\n```";
    let msg = Message {
        role: ROLE_ASSISTANT.to_string(),
        content: content.to_string(),
    };

    let rendered = render_message(&msg, &Theme::dark_default(), true, false);

    // Extract unique block indices
    let indices: Vec<usize> = rendered.span_metadata.iter()
        .flat_map(|line| line.iter())
        .filter_map(|k| k.code_block_meta().map(|m| m.block_index()))
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    assert_eq!(indices.len(), 2);
    assert!(indices.contains(&0));
    assert!(indices.contains(&1));
}

#[test]
fn empty_code_block_emits_metadata() {
    // Edge case: ```\n```
    let content = "```\n```";
    let msg = Message {
        role: ROLE_ASSISTANT.to_string(),
        content: content.to_string(),
    };

    let rendered = render_message(&msg, &Theme::dark_default(), true, false);

    // Even empty blocks should have metadata on the containing line
    let has_code_meta = rendered.span_metadata.iter()
        .flat_map(|line| line.iter())
        .any(|k| k.is_code_block());

    // Note: Empty blocks might not create content spans, but should
    // be tracked for block navigation purposes
    // This test documents the expected behavior
    assert!(has_code_meta || rendered.code_block_indices.contains(&0),
            "Empty blocks should be tracked");
}
```

**Deliverable**: Renderer emitting code block metadata with 3+ passing tests

**Phase 1 Success Criteria**:
- ✅ `SpanKind::CodeBlock` variant exists and compiles
- ✅ `CodeBlockMeta` stores language and index
- ✅ Markdown renderer emits metadata during code block rendering
- ✅ All Phase 0 and Phase 1 tests pass
- ✅ `cargo clippy` reports no new warnings

---

### Phase 2: Cache Integration (Size: M)

**Goal**: Ensure code block metadata is cached and reused efficiently

#### Task 2.1: Verify Metadata Flows Through Cache (Size: S)

Test that existing cache infrastructure works with code block metadata:

```rust
#[test]
fn prewrapped_metadata_includes_code_blocks() {
    let mut app = create_test_app();
    app.ui.messages.push_back(fixtures::single_block());

    let width = 80u16;
    let metadata = app.get_prewrapped_span_metadata_cached(width);

    let has_code_blocks = metadata.iter()
        .flat_map(|line| line.iter())
        .any(|k| k.is_code_block());

    assert!(has_code_blocks, "Cached metadata should include code blocks");
}

#[test]
fn cache_preserves_code_block_indices() {
    let mut app = create_test_app();
    app.ui.messages.push_back(fixtures::multiple_blocks());

    let width = 80u16;

    // Cache metadata twice
    let meta1 = app.get_prewrapped_span_metadata_cached(width);
    let meta2 = app.get_prewrapped_span_metadata_cached(width);

    // Should return same Arc
    assert!(Arc::ptr_eq(meta1, meta2), "Should reuse cached metadata");

    // Extract indices from first cache
    let indices1: Vec<usize> = extract_code_block_indices(meta1);
    let indices2: Vec<usize> = extract_code_block_indices(meta2);

    assert_eq!(indices1, indices2, "Indices should be stable across cache hits");
}
```

**Deliverable**: 2-3 tests verifying cache integration

#### Task 2.2: Add Helper for Code Block Extraction (Size: XS)

Add utility in `src/ui/span.rs` or `src/utils/scroll.rs`:

```rust
/// Extracts code blocks from span metadata.
///
/// Returns a vector of (line_index, block_index, language) tuples
/// identifying each code block's position in the rendered output.
///
/// # Arguments
///
/// * `lines` - Rendered lines from the layout
/// * `metadata` - Span metadata parallel to lines
///
/// # Returns
///
/// Vector of code block positions sorted by appearance order.
pub fn extract_code_blocks(
    lines: &[Line],
    metadata: &[Vec<SpanKind>],
) -> Vec<CodeBlockPosition> {
    let mut blocks: HashMap<usize, CodeBlockPosition> = HashMap::new();

    for (line_idx, line_meta) in metadata.iter().enumerate() {
        for span_kind in line_meta {
            if let Some(meta) = span_kind.code_block_meta() {
                blocks.entry(meta.block_index())
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

/// Position of a code block in rendered output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeBlockPosition {
    /// Zero-based block index.
    pub block_index: usize,
    /// First line of the block (inclusive).
    pub start_line: usize,
    /// Last line of the block (inclusive).
    pub end_line: usize,
    /// Language tag if specified.
    pub language: Option<String>,
}
```

**Tests**:
```rust
#[test]
fn extract_code_blocks_finds_all_blocks() {
    let msg = fixtures::multiple_blocks();
    let rendered = render_message(&msg, &Theme::dark_default(), true, false);

    let positions = extract_code_blocks(&rendered.lines, &rendered.span_metadata);

    assert_eq!(positions.len(), 3);
    assert_eq!(positions[0].block_index, 0);
    assert_eq!(positions[1].block_index, 1);
    assert_eq!(positions[2].block_index, 2);
}

#[test]
fn extract_code_blocks_computes_line_ranges() {
    let msg = fixtures::single_block();
    let rendered = render_message(&msg, &Theme::dark_default(), true, false);

    let positions = extract_code_blocks(&rendered.lines, &rendered.span_metadata);

    assert_eq!(positions.len(), 1);
    let block = &positions[0];
    assert!(block.start_line < block.end_line, "Block should span multiple lines");
}
```

**Deliverable**: Helper function with 2-3 passing tests

**Phase 2 Success Criteria**:
- ✅ Code block metadata flows through cache
- ✅ Helper extracts code blocks from metadata
- ✅ All tests pass
- ✅ No cache invalidation regressions

---

### Phase 3: Block Selection Simplification (Size: M)

**Goal**: Replace code block recomputation with cached metadata queries

#### Task 3.1: Update Block Selection Event Handler (Size: S)

Modify `src/ui/chat_loop/modes.rs:handle_block_select_mode_event()`:

```rust
pub async fn handle_block_select_mode_event(
    app: &AppHandle,
    key: &event::KeyEvent,
    term_width: u16,
    term_height: u16,
) -> bool {
    app.update(|app| {
        if !app.ui.in_block_select_mode() {
            return false;
        }

        // Query cached metadata instead of recomputing
        let metadata = app.get_prewrapped_span_metadata_cached(term_width);
        let lines = app.get_prewrapped_lines_cached(term_width);
        let blocks = crate::ui::span::extract_code_blocks(lines, metadata);

        match key.code {
            KeyCode::Esc => {
                app.ui.exit_block_select_mode();
                true
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(cur) = app.ui.selected_block_index() {
                    let total = blocks.len();
                    if let Some(next) = wrap_previous_index(cur, total) {
                        app.ui.set_selected_block_index(next);
                        if let Some(block) = blocks.get(next) {
                            scroll_block_into_view(app, term_width, term_height, block.start_line);
                        }
                    }
                }
                true
            }
            // ... similar updates for Down, Copy, Save ...
        }
    })
    .await
}
```

**Tests**:
```rust
#[test]
fn block_select_navigation_uses_cache() {
    let runtime = Runtime::new().unwrap();
    runtime.block_on(async {
        let handle = setup_app_with_blocks();

        // Enter block select mode
        handle.update(|app| {
            app.ui.enter_block_select_mode(0);
        }).await;

        // Simulate Up key
        let key = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        let handled = handle_block_select_mode_event(&handle, &key, 80, 24).await;

        assert!(handled);

        // Verify cache was used (block_index changed)
        let selected = handle.read(|app| app.ui.selected_block_index()).await;
        assert!(selected.is_some());
    });
}
```

**Deliverable**: Updated handler using cached metadata with 2+ tests

#### Task 3.2: Update Ctrl+B Handler (Size: S)

Modify `src/ui/chat_loop/keybindings/handlers.rs:CtrlBHandler`:

```rust
impl KeyHandler for CtrlBHandler {
    async fn handle(&self, app: &AppHandle, ...) -> KeyResult {
        app.update(|app| {
            if !app.ui.markdown_enabled {
                app.conversation().set_status("Markdown disabled (/markdown on)");
                return KeyResult::Handled;
            }

            // Use cached metadata
            let metadata = app.get_prewrapped_span_metadata_cached(term_width);
            let lines = app.get_prewrapped_lines_cached(term_width);
            let blocks = crate::ui::span::extract_code_blocks(lines, metadata);

            if app.ui.in_block_select_mode() {
                // Navigate to previous block
                if let Some(cur) = app.ui.selected_block_index() {
                    let total = blocks.len();
                    if let Some(next) = wrap_previous_index(cur, total) {
                        app.ui.set_selected_block_index(next);
                        if let Some(block) = blocks.get(next) {
                            scroll_block_into_view(app, term_width, term_height, block.start_line);
                        }
                    }
                }
            } else if blocks.is_empty() {
                app.conversation().set_status("No code blocks");
            } else {
                let last = blocks.len().saturating_sub(1);
                app.ui.enter_block_select_mode(last);
                if let Some(block) = blocks.get(last) {
                    scroll_block_into_view(app, term_width, term_height, block.start_line);
                }
            }

            KeyResult::Handled
        }).await
    }
}
```

**Deliverable**: Updated Ctrl+B handler

#### Task 3.3: Update Renderer to Use Cached Metadata (Size: S)

Modify `src/ui/renderer.rs` to use cached metadata even in block select mode:

```rust
pub fn ui(f: &mut Frame, app: &mut App) {
    // ...

    let (lines, span_metadata) = if app.ui.in_block_select_mode() {
        // Can now use cache with highlighting applied in-place
        let mut lines = app.get_prewrapped_lines_cached(chunks[0].width).as_ref().clone();
        let metadata = app.get_prewrapped_span_metadata_cached(chunks[0].width);

        if let Some(selected_idx) = app.ui.selected_block_index() {
            apply_code_block_highlight(&mut lines, metadata, selected_idx);
        }

        (lines, metadata.as_ref().clone())
    } else if app.ui.in_edit_select_mode() {
        // ... existing logic ...
    } else {
        // Normal mode: use cache directly
        let lines = app.get_prewrapped_lines_cached(chunks[0].width).clone();
        let metadata = app.get_prewrapped_span_metadata_cached(chunks[0].width).clone();
        (lines, metadata)
    };

    // ...
}

/// Applies highlighting to spans belonging to a specific code block.
fn apply_code_block_highlight(
    lines: &mut [Line],
    metadata: &[Vec<SpanKind>],
    block_index: usize,
) {
    for (line, line_meta) in lines.iter_mut().zip(metadata.iter()) {
        for (span, kind) in line.spans.iter_mut().zip(line_meta.iter()) {
            if let Some(meta) = kind.code_block_meta() {
                if meta.block_index() == block_index {
                    span.style = span.style.add_modifier(Modifier::BOLD);
                }
            }
        }
    }
}
```

**Tests**:
```rust
#[test]
fn renderer_uses_cache_in_block_select_mode() {
    let mut app = create_test_app();
    app.ui.messages.push_back(fixtures::multiple_blocks());
    app.ui.enter_block_select_mode(1);

    // Prime cache
    let width = 80u16;
    let _first = app.get_prewrapped_lines_cached(width);

    // Render in block select mode
    // (In real code this would call ui(), we'll test the helper)
    let mut lines = app.get_prewrapped_lines_cached(width).as_ref().clone();
    let metadata = app.get_prewrapped_span_metadata_cached(width);

    apply_code_block_highlight(&mut lines, metadata, 1);

    // Verify highlighting applied
    let has_bold = lines.iter().any(|line| {
        line.spans.iter().any(|s| s.style.add_modifier.contains(Modifier::BOLD))
    });

    assert!(has_bold, "Selected block should be highlighted");
}
```

**Deliverable**: Renderer using cached metadata with highlighting helper

**Phase 3 Success Criteria**:
- ✅ Block selection queries cache instead of re-rendering
- ✅ Ctrl+B handler uses cached metadata
- ✅ Renderer uses cache in block select mode
- ✅ All navigation tests pass
- ✅ No performance regressions (verify with manual testing)

---

### Phase 4: Content Extraction (Size: S)

**Goal**: Extract code block content from rendered output using metadata

#### Task 4.1: Add Content Extraction Helper (Size: S)

```rust
/// Extracts the text content of a code block.
///
/// Concatenates all spans belonging to the specified code block,
/// preserving line breaks between lines.
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
pub fn extract_code_block_content(
    lines: &[Line],
    metadata: &[Vec<SpanKind>],
    block_index: usize,
) -> Option<String> {
    let mut content = String::new();
    let mut found_any = false;

    for (line, line_meta) in lines.iter().zip(metadata.iter()) {
        let mut line_content = String::new();

        for (span, kind) in line.spans.iter().zip(line_meta.iter()) {
            if let Some(meta) = kind.code_block_meta() {
                if meta.block_index() == block_index {
                    line_content.push_str(&span.content);
                    found_any = true;
                }
            }
        }

        if !line_content.is_empty() {
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
```

**Tests**:
```rust
#[test]
fn extract_code_block_content_retrieves_code() {
    let msg = fixtures::single_block();
    let rendered = render_message(&msg, &Theme::dark_default(), true, false);

    let content = extract_code_block_content(
        &rendered.lines,
        &rendered.span_metadata,
        0,
    );

    assert!(content.is_some());
    let code = content.unwrap();
    assert!(code.contains("fn main()"), "Should extract function content");
}

#[test]
fn extract_preserves_line_breaks() {
    let msg = Message {
        role: ROLE_ASSISTANT.to_string(),
        content: "```rust\nline1\nline2\nline3\n```".to_string(),
    };
    let rendered = render_message(&msg, &Theme::dark_default(), true, false);

    let content = extract_code_block_content(
        &rendered.lines,
        &rendered.span_metadata,
        0,
    ).unwrap();

    let line_count = content.lines().count();
    assert_eq!(line_count, 3, "Should preserve 3 lines");
}

#[test]
fn extract_returns_none_for_invalid_index() {
    let msg = fixtures::single_block();
    let rendered = render_message(&msg, &Theme::dark_default(), true, false);

    let content = extract_code_block_content(
        &rendered.lines,
        &rendered.span_metadata,
        999,
    );

    assert!(content.is_none(), "Invalid index should return None");
}
```

**Deliverable**: Content extraction helper with 3+ tests

#### Task 4.2: Update Copy/Save Handlers (Size: S)

Update block selection handlers in `src/ui/chat_loop/modes.rs`:

```rust
KeyCode::Char('c') | KeyCode::Char('C') => {
    if let Some(cur) = app.ui.selected_block_index() {
        let metadata = app.get_prewrapped_span_metadata_cached(term_width);
        let lines = app.get_prewrapped_lines_cached(term_width);

        if let Some(content) = extract_code_block_content(lines, metadata, cur) {
            match crate::utils::clipboard::copy_to_clipboard(&content) {
                Ok(()) => app.conversation().set_status("Copied code block"),
                Err(_) => app.conversation().set_status("Clipboard error"),
            }
            app.ui.exit_block_select_mode();
        }
    }
    true
}

KeyCode::Char('s') | KeyCode::Char('S') => {
    if let Some(cur) = app.ui.selected_block_index() {
        let metadata = app.get_prewrapped_span_metadata_cached(term_width);
        let lines = app.get_prewrapped_lines_cached(term_width);
        let blocks = extract_code_blocks(lines, metadata);

        if let Some(block) = blocks.get(cur) {
            if let Some(content) = extract_code_block_content(lines, metadata, cur) {
                let ext = language_to_extension(block.language.as_deref());
                let date = Utc::now().format("%Y-%m-%d");
                let filename = format!("chabeau-block-{}.{}", date, ext);

                if std::path::Path::new(&filename).exists() {
                    app.ui.start_file_prompt_save_block(filename, content);
                } else {
                    match fs::write(&filename, &content) {
                        Ok(()) => app.conversation().set_status(format!("Saved to {}", filename)),
                        Err(_) => app.conversation().set_status("Error saving code block"),
                    }
                }
                app.ui.exit_block_select_mode();
            }
        }
    }
    true
}
```

**Tests**:
```rust
#[test]
fn copy_handler_extracts_correct_content() {
    let runtime = Runtime::new().unwrap();
    runtime.block_on(async {
        let handle = setup_app_with_blocks();

        handle.update(|app| {
            app.ui.enter_block_select_mode(0);
        }).await;

        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE);
        let handled = handle_block_select_mode_event(&handle, &key, 80, 24).await;

        assert!(handled);

        // Verify status message indicates success
        let status = handle.read(|app| app.ui.status.clone()).await;
        assert_eq!(status, Some("Copied code block".to_string()));
    });
}
```

**Deliverable**: Updated copy/save handlers with tests

**Phase 4 Success Criteria**:
- ✅ Content extraction works for all test fixtures
- ✅ Copy handler uses metadata
- ✅ Save handler uses metadata
- ✅ All edge cases handled (empty blocks, invalid indices)

---

### Phase 5: Cleanup and Documentation (Size: S)

**Goal**: Remove obsolete code and document the new architecture

#### Task 5.1: Remove `compute_codeblock_ranges()` Functions (Size: XS)

Remove from `src/ui/markdown.rs`:
- `compute_codeblock_ranges()`
- `compute_codeblock_ranges_with_width_and_policy()`
- `compute_codeblock_contents_with_lang()`

Verify no remaining callers:
```bash
rg "compute_codeblock_ranges" --type rust
```

**Deliverable**: Obsolete functions removed, no compiler errors

#### Task 5.2: Remove `build_layout_with_codeblock_highlight_and_flags_and_width()` (Size: XS)

Remove from `src/utils/scroll.rs`:
- Special-case code block highlighting layout builder

Update `src/ui/renderer.rs` to use the simpler cache-based approach from Phase 3.

**Deliverable**: Obsolete layout builder removed

#### Task 5.3: Add Module Documentation (Size: XS)

Add to `src/ui/span.rs`:

```rust
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
//! ```rust
//! use crate::ui::span::{SpanKind, CodeBlockMeta};
//!
//! // Create code block metadata
//! let kind = SpanKind::code_block(Some("rust"), 0);
//!
//! // Query metadata
//! if let Some(meta) = kind.code_block_meta() {
//!     println!("Language: {:?}", meta.language());
//! }
//! ```
```

Add function-level documentation to all new helpers:
- `extract_code_blocks()`
- `extract_code_block_content()`
- `apply_code_block_highlight()`

**Deliverable**: Comprehensive documentation following AGENTS.md guidelines

#### Task 5.4: Run Full Test Suite and Linters (Size: XS)

```bash
cargo test
cargo test --doc
cargo check
cargo fmt
cargo clippy
```

Verify:
- ✅ All unit tests pass (540+ tests)
- ✅ All doc tests pass
- ✅ No clippy warnings
- ✅ Code formatted correctly

**Deliverable**: Clean test and lint output

**Phase 5 Success Criteria**:
- ✅ Obsolete code removed
- ✅ No dead code warnings
- ✅ Comprehensive documentation added
- ✅ All tests and lints pass

---

## Edge Cases and Test Coverage

### Edge Case Matrix

| Scenario | Test Fixture | Phase | Status |
|----------|--------------|-------|--------|
| Single code block | `fixtures::single_block()` | 0, 1 | ✅ |
| Multiple blocks in message | `fixtures::multiple_blocks()` | 0, 1 | ✅ |
| Blocks across messages | `fixtures::multiple_messages()` | 0, 2 | ✅ |
| Nested in ordered list | `fixtures::nested_in_list()` | 0, 1 | ✅ |
| Long lines with wrapping | `fixtures::wrapped_code()` | 0, 4 | ✅ |
| Empty code block | `fixtures::empty_block()` | 0, 1 | ✅ |
| No language tag | `fixtures::no_language()` | 0, 1 | ✅ |
| Adjacent to text | `fixtures::adjacent_to_text()` | 0, 1 | ✅ |
| User vs assistant message | Both roles tested | 0 | ✅ |
| Syntax highlighting on/off | Both modes tested | 1 | ✅ |
| Cache invalidation | Width change, content change | 2 | ✅ |
| Invalid block index | Extraction with index > count | 4 | ✅ |
| Copy/save operations | All block types | 4 | ✅ |

### Performance Benchmarks (Optional)

Add to `benches/span_metadata.rs`:

```rust
fn bench_block_selection_navigation(c: &mut Criterion) {
    let mut group = c.benchmark_group("block_selection");

    group.bench_function("navigate_with_metadata", |b| {
        let app = setup_app_with_many_blocks();
        b.iter(|| {
            // Navigate through all blocks using cached metadata
            for i in 0..10 {
                let _ = navigate_to_block(&app, i);
            }
        });
    });

    group.finish();
}
```

**Target**: Block navigation should be <1ms per keypress with cached metadata

---

## Success Metrics

### Quantitative Goals

- ✅ All Phase 0 test fixtures implemented (10 fixtures)
- ✅ All failing tests from Phase 0 now pass (15+ tests)
- ✅ Code block metadata cached and reused (verified in Phase 2)
- ✅ 100+ lines of obsolete code removed (Phase 5)
- ✅ Zero compiler warnings
- ✅ Zero clippy warnings
- ✅ All 540+ unit tests pass
- ✅ `cargo doc` generates without warnings

### Qualitative Goals

- ✅ Block navigation feels instant (no visible lag)
- ✅ Architecture consistent with link metadata
- ✅ Code is self-documenting with clear comments
- ✅ No breadcrumb comments about old implementation (per AGENTS.md)
- ✅ Future extensions are straightforward (hover, inline expand, etc.)

### Performance Validation

**Before** (current implementation):
- Block navigation: ~5-10ms per keypress (re-renders all messages)
- Enter block select mode: ~10-20ms (full layout computation)

**After** (with metadata):
- Block navigation: <1ms per keypress (cache query)
- Enter block select mode: <1ms (cache query)

**Verification**:
```bash
cargo bench --bench span_metadata
```

---

## Risk Mitigation

### Risk 1: Cache Invalidation Bugs
**Mitigation**: Comprehensive tests for cache invalidation scenarios (Phase 2)
**Detection**: Monitor cache hit rates in testing

### Risk 2: Metadata Size Impact
**Mitigation**: `CodeBlockMeta` uses `Arc<str>` for language (shared allocation)
**Detection**: Measure memory usage in benchmarks

### Risk 3: Breaking Changes to Render Pipeline
**Mitigation**: Incremental phases with continuous testing
**Detection**: Run full test suite after each task

### Risk 4: Edge Cases in Content Extraction
**Mitigation**: 10+ edge case fixtures defined upfront (Phase 0)
**Detection**: Exhaustive test coverage before implementation

---

## Future Extensions Enabled

With code block metadata in place, future enhancements become straightforward:

1. **Hover Previews**: Show language and line count on hover
2. **Inline Expansion**: Collapse/expand long code blocks
3. **Syntax Theme Switching**: Per-block syntax highlighting
4. **Search Within Blocks**: Filter blocks by language or content
5. **Export All Blocks**: Batch export with preserved languages
6. **Block Annotations**: Add notes or tags to specific blocks

All of these can query `SpanKind::CodeBlock` metadata without renderer changes.

---

## References

### Related Files

**Span system**:
- `src/ui/span.rs` - Span metadata types
- `src/ui/markdown.rs` - Markdown to span rendering

**Block selection**:
- `src/ui/chat_loop/modes.rs` - Block select event handling
- `src/ui/chat_loop/keybindings/handlers.rs` - Ctrl+B handler

**Rendering**:
- `src/ui/renderer.rs` - Terminal UI rendering
- `src/utils/scroll.rs` - Layout computation and caching

### Similar Patterns

**Link metadata**: `SpanKind::Link(LinkMeta)` provides a template for code blocks
**Cache usage**: `App::get_prewrapped_span_metadata_cached()` demonstrates the caching pattern

---

## Implementation Checklist

### Phase 0: Test Infrastructure
- [ ] Task 0.1: Define 10 test fixtures
- [ ] Task 0.2: Write 8-10 failing tests for span metadata
- [ ] Task 0.3: Write 3-5 failing tests for cache behavior

### Phase 1: Core Span Metadata Extension
- [ ] Task 1.1: Define `CodeBlockMeta` type
- [ ] Task 1.2: Add `CodeBlock` variant to `SpanKind`
- [ ] Task 1.3: Update markdown renderer to emit metadata

### Phase 2: Cache Integration
- [ ] Task 2.1: Verify metadata flows through cache
- [ ] Task 2.2: Add helper for code block extraction

### Phase 3: Block Selection Simplification
- [ ] Task 3.1: Update block selection event handler
- [ ] Task 3.2: Update Ctrl+B handler
- [ ] Task 3.3: Update renderer to use cached metadata

### Phase 4: Content Extraction
- [ ] Task 4.1: Add content extraction helper
- [ ] Task 4.2: Update copy/save handlers

### Phase 5: Cleanup and Documentation
- [ ] Task 5.1: Remove `compute_codeblock_ranges()` functions
- [ ] Task 5.2: Remove obsolete layout builder
- [ ] Task 5.3: Add module documentation
- [ ] Task 5.4: Run full test suite and linters

### Final Verification
- [ ] All 540+ unit tests pass
- [ ] All new doc tests pass
- [ ] Zero compiler warnings
- [ ] Zero clippy warnings
- [ ] Performance benchmarks meet targets
- [ ] Manual testing in TUI confirms smooth navigation

---

**End of Plan**

---

## Post-Implementation Notes

### Bugs Discovered and Fixed

#### Bug 1: Per-Message Block Indices (Not Globally Unique)

**Discovered**: During initial user testing after Phase 3 completion  
**Symptom**: Navigation cycled through code blocks but multiple blocks were highlighted simultaneously  

**Root Cause**: Block indices were assigned per-message, not globally:
- Each message's `MarkdownRenderer` started counting from 0
- Message 1 had blocks [0, 1, 2], Message 2 had blocks [0, 1, 2]
- `extract_code_blocks()` used `HashMap<usize, CodeBlockPosition>` keyed by block_index
- Duplicate indices overwrote each other in the HashMap
- Result: Only one block per index survived, and selecting index 0 highlighted ALL blocks with index 0

**Fix**: Added global block index renumbering in `LayoutEngine::layout_messages()` (commit `daf910d`)
- Track `global_block_index` across all messages
- After rendering each message, detect its code blocks
- Renumber them from per-message indices to sequential global indices
- Sort local indices before mapping to ensure deterministic ordering

**Test Coverage**: `test_blocks_across_messages_have_unique_indices`

#### Bug 2: Incremental Cache Updates Bypassed Global Renumbering

**Discovered**: During further testing with real conversation flow  
**Symptom**: After receiving assistant response with code, pressing Ctrl+B highlighted BOTH the old and new code blocks

**Root Cause**: Two code paths for building cache:
1. **Full rebuild** (`LayoutEngine::layout_messages()`) - correctly applied global renumbering ✅
2. **Incremental update** (`splice_last_message_layout()`) - bypassed renumbering ❌

When only the last message changed:
- Incremental path rendered ONLY that message with per-message indices (starting at 0)
- Spliced it directly into cache without renumbering
- Created duplicate indices: cache had [0, 1, 0] instead of [0, 1, 2]
- Selecting index 0 highlighted both blocks with that index

**Fix**: Updated `splice_last_message_layout()` to preserve global uniqueness (commit `9d6ba26`)
- Find maximum existing block index in the cache before splice
- Renumber new message's block indices with offset `max + 1`
- Guarantees global uniqueness even in incremental updates

**Test Coverage**: `test_incremental_cache_update_preserves_global_indices`

### Why Tests Didn't Catch These Bugs Initially

**Phase 0 tests** used fixtures like `multiple_blocks()` - a **single message** with multiple blocks:
- Triggered full rebuild path only
- Never tested incremental updates
- Never tested blocks across separate messages

**Lessons Learned**:
- Test both code paths: full rebuild AND incremental update
- Test realistic conversation flows: multiple back-and-forth exchanges
- Test edge cases: blocks in non-adjacent messages, interleaved with non-code messages

### Final State

- All 581 tests passing (includes 14 previously-ignored Phase 0 tests)
- Block navigation works correctly across any conversation structure
- Global block indices maintained correctly in both rebuild and incremental update paths
- No performance regressions - caching works as designed
