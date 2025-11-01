# Complex input area refactor plan

## Problem statement
- Multi-paragraph pastes expose gaps in the wrapped cursor layout. Consecutive newlines do not receive dedicated line entries, so vertical navigation stops at hard newlines and the cursor column tracking desynchronizes.
- Home/End helpers and the up/down helpers recompute wrap information ad-hoc and lose the user's preferred column, which causes jitter once the textarea reflows around large edits.
- Layout computations happen repeatedly on every navigation key, producing avoidable flicker under heavy text because we rebuild the wrap structure from scratch.

## Goals
1. Produce a single source of truth for wrapped input layout that properly accounts for both soft and hard line breaks (including blank paragraphs).
2. Let all cursor helpers (up/down, page-up/down, home/end) read from that shared layout and preserve the preferred column across repeated calls.
3. Keep the textarea state and the layout cache in sync after any edit, ensuring pastes leave the cursor at the end of the inserted text and the UI stays responsive.
4. Cover regressions with focused tests around blank paragraphs, Ctrl+A/Ctrl+E/Home/End, and multi-line pastes.

## High-level approach
- Replace the current `TextWrapper::cursor_layout` implementation with a streaming layout builder that walks the original input once. It will:
  - Track visual line/column coordinates while emitting wrapped text.
  - Insert explicit zero-width line entries for blank paragraphs so the cursor can move "through" hard newlines.
  - Return both the wrapped string and a `WrappedCursorLayout` built from the same traversal to eliminate divergence between wrapping and cursor math.
- Expose a `wrap_with_layout` helper returning `{ wrapped_text, layout }` to DRY up `wrap_text`, `cursor_layout`, and map builders.
- Add an `InputLayoutCache` struct in `UiState` keyed by `(wrap_width, revision)` that stores the last `WrappedCursorLayout`. Increment an `input_revision` counter whenever the text actually changes and invalidate the cache on width changes.
- Update cursor movement helpers to fetch the cached layout via a new `ensure_wrapped_cursor_layout(width)` method instead of recomputing. Preserve `input_cursor_preferred_column` when movement fails so repeated attempts stay aligned.
- After paste operations (and any edit that inserts text) ensure we set the cursor to the end and refresh the layout cache.

## Testing strategy
- Extend unit tests in `core/app/tests.rs` to cover:
  - Moving up/down across double newlines with varying wrap widths.
  - `move_cursor_to_visual_line_start` / `move_cursor_to_visual_line_end` when the cursor is adjacent to a blank line.
  - Large paste behavior: the cursor lands at the end and vertical navigation immediately after the paste crosses paragraph boundaries.
- Add targeted tests for the new layout builder in `core/text_wrapping.rs`, verifying that blank lines appear in the position map and that line counts are monotonic.
- Run the standard formatting and lint commands plus the full test suite.
