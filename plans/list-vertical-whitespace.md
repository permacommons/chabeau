# List Vertical Whitespace Plan

## Problem Statement

When markdown source contains blank lines between list items (at any nesting level), those blank lines currently render in the **wrong location**.

### Concrete Example

**Source Markdown:**
```markdown
- Strategic Foundations
  - Long-Horizon Thinking
    - Scenario Branches

- Implementation Patterns
  - Knowledge Architecture
    - Modular repositories

- Resilience
  - Stressors
```

Note: There is a blank line in the source BEFORE "Implementation Patterns" and BEFORE "Resilience".

**Buggy Rendering:**
```
Line 0: - Strategic Foundations
Line 1: [BLANK]                      ← Wrong! This shouldn't be here
Line 2:   - Long-Horizon Thinking
Line 3:     - Scenario Branches
Line 4: - Implementation Patterns     ← Should have blank line BEFORE this
Line 5: [BLANK]                      ← Wrong! This shouldn't be here
Line 6:   - Knowledge Architecture
Line 7:     - Modular repositories
Line 8: - Resilience                 ← Should have blank line BEFORE this
```

**Correct Rendering:**
```
Line 0: - Strategic Foundations
Line 1:   - Long-Horizon Thinking
Line 2:     - Scenario Branches
Line 3: [BLANK]                      ← Correct! Matches source blank line
Line 4: - Implementation Patterns
Line 5:   - Knowledge Architecture
Line 6:     - Modular repositories
Line 7: [BLANK]                      ← Correct! Matches source blank line
Line 8: - Resilience
Line 9:   - Stressors
```

### Summary of Bug

**Current behavior:** Blank lines appear AFTER each list item's paragraph content, creating visual separation between parent items and their children.

**Desired behavior:** Blank lines should appear BEFORE list items that had blank lines before them in the source, preserving the document's visual structure.

## Root Cause Analysis

### Parser Behavior

The pulldown-cmark parser treats blank lines in list context specially, but doesn't expose information about where those blank lines were in the source. We can't determine from parser events alone which items should have blank lines before them.

### Current Renderer Logic

The `TagEnd::Paragraph` handler adds a blank line after every paragraph. Since list items contain paragraphs, this creates blank lines after list item content:

```rust
TagEnd::Paragraph => {
    self.flush_current_spans(true);
    self.push_empty_line();  // ← Added after every paragraph
}
```

This causes unwanted blank lines throughout lists, regardless of source structure.

## Implemented Solution

### Chosen Approach: Source Preprocessing + Absolute Item Indexing

The elegant solution is to:
1. **Preprocess** the source markdown to find which items need blank lines before them
2. Track items by **absolute position** in the document (0, 1, 2, 3...) regardless of nesting level
3. Add blank lines during rendering based on the preprocessed set

### Implementation Details

**Step 1: Preprocessing** (`find_items_needing_blank_lines`)

Scan the source markdown line-by-line to detect list items preceded by blank lines:

```rust
fn find_items_needing_blank_lines(content: &str) -> std::collections::HashSet<usize> {
    let mut result = std::collections::HashSet::new();
    let mut item_index = 0;
    let mut prev_was_blank = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Check if this is a list item at any indentation level
        let is_list_item = trimmed.starts_with("- ") ||
                           trimmed.starts_with("* ") ||
                           trimmed.chars().next().is_some_and(|c| c.is_ascii_digit());

        if is_list_item {
            if prev_was_blank && item_index > 0 {
                result.insert(item_index);  // This item needs a blank line before it
            }
            item_index += 1;
            prev_was_blank = false;
        } else if trimmed.is_empty() {
            prev_was_blank = true;
        }
    }

    result
}
```

**Key aspects:**
- Detects items at **any indentation level** (by checking `trimmed` which strips leading whitespace)
- Tracks items sequentially: first item = 0, second item = 1, etc.
- Records indices of items that had blank lines before them in the source

**Step 2: State Tracking**

Add two fields to `MarkdownRenderer`:
```rust
/// Track which list items should have blank lines before them (by absolute position)
items_needing_blank_lines_before: std::collections::HashSet<usize>,
/// Current item index (increments for every list item encountered)
current_item_index: usize,
```

**Step 3: Rendering**

Check the set when rendering each item:
```rust
Tag::Item => {
    // Check if this item needs a blank line before it
    if self.items_needing_blank_lines_before.contains(&self.current_item_index) {
        self.push_empty_line();
    }
    self.current_item_index += 1;
    // ... continue with normal item rendering
}
```

**Step 4: Fix Paragraph Spacing**

Don't add blank lines after paragraphs that are inside lists:
```rust
TagEnd::Paragraph => {
    self.flush_current_spans(true);
    // Only add blank line if NOT inside a list
    if self.list_stack.is_empty() {
        self.push_empty_line();
    }
}
```

### Why This Approach is Elegant

1. **Simple state:** Just a HashSet and a counter
2. **Handles all nesting levels:** Automatically works for nested items because we track by absolute position
3. **Minimal code:** ~25 lines for preprocessing, ~4 lines for rendering check
4. **Source of truth:** The source markdown itself tells us where blank lines should be
5. **No event timing issues:** Preprocessing happens once upfront, rendering just checks the set

### Example Walkthrough

Source:
```markdown
- Item 0

- Item 1
  - Item 2
```

Preprocessing:
- Line 1: "- Item 0" → item_index=0, prev_was_blank=false
- Line 2: "" → prev_was_blank=true
- Line 3: "- Item 1" → prev_was_blank=true, so record index 1, then item_index=1
- Line 4: "  - Item 2" → item_index=2, prev_was_blank=false
- Result: `{1}` (only item 1 needs blank line before it)

Rendering:
- Tag::Item fires, current_item_index=0 → not in set, no blank line added, increment to 1
- Tag::Item fires, current_item_index=1 → in set! add blank line, increment to 2
- Tag::Item fires, current_item_index=2 → not in set, no blank line added, increment to 3

Output:
```
- Item 0
[BLANK]
- Item 1
  - Item 2
```

## Testing Strategy

Comprehensive tests cover:
- ✅ Top-level items with blank lines between them
- ✅ Top-level items without blank lines (tight lists)
- ✅ Nested items with blank lines at various depths
- ✅ Nested items without blank lines (tight nested lists)
- ✅ Complex nesting: ordered → bullets → sub-bullets
- ✅ Long text that wraps across lines
- ✅ Lists preceded/followed by paragraphs and headings

All 48 markdown tests pass.

## Success Criteria

✅ **All achieved:**
1. Blank lines appear BEFORE items that had them in source
2. No blank lines appear AFTER list item content
3. Works at all nesting levels
4. Preserves parent-child visual grouping
5. No regression in existing tests
6. Clean, maintainable implementation
