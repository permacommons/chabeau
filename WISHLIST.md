# Chabeau Wishlist

This file contains features and improvements that would be nice to have in future versions of Chabeau.

## Features

- More complete keyboard mapping (Page Up/Dn; Ctrl+D)
- Tiny copy/paste affordances
- Better handling of repeating messages like "Generating..."
- In-TUI provider selector
- In-TUI default picker
- Tab completion for commands
- Support common "character cards"
- Rapid initialization of "modes" like "Concise command help" or "Act as a reviewer for" 
- Make assistant messages editable (may require further rethinking of input area)
- Basic "push a file into context" support
- "Rapid refine" - apply a previously created prompt to an output
- Microphone/speaker support?

### Rendering
- Configurable syntax highlighting theme (pick a syntect theme or a color preset)
- Consider auto-detecting language for fenced code blocks without a language tag

## Tests

- Performance regression tests

## Code quality

- Refactor `src/ui/chat_loop.rs` into smaller, testable units
  - Extract input handling, selection mode, picker handling, and streaming dispatch into helpers/modules
  - Introduce a single `Mode` enum (e.g., `Typing | EditSelect | InPlaceEdit { index }`) to replace multiple booleans
  - Add pure functions for selection navigation (wrap-around) and reuse across keys (↑/↓/j/k/Ctrl+P)
- Unify message line building in `src/utils/scroll.rs`
  - Merge normal and highlighted builders behind a single function with an optional style patch
  - Consider full-row background highlight for selected messages (not just styled spans)
  - Reduce duplication in `src/ui/markdown.rs` for code block flushing (extract helper)
  - Consolidate plain vs markdown rendering path selection
- Centralize help text
  - Create a small `ui/help.rs` with canonical key-hint strings used by CLI long_about, `/help`, and renderer titles
- Theme token for selection highlight
  - Add a dedicated `selection_highlight` color/style to theme specs and built-ins
  - Update renderer to use it instead of reusing `streaming_indicator_style`
- Logging durability
  - Make log rewrites (after truncate/in-place edit) atomic via temp file + rename
  - Optionally append a log marker indicating manual history edits
- Height/scroll DRYing
  - Prefer `App::calculate_available_height` everywhere; keep a single helper for “scroll selected index into view” (already added)
- Stream spawning DRYing
  - Use a config struct (already added) and consider moving the helper out of `chat_loop` for reuse
- Tests
  - Add unit tests for selection wrap-around and mode transitions (pure helpers)
  - Add integration-style tests for event handling if feasible (simulate key events)
  - Add tests for `/markdown` and `/syntax` commands by injecting a test config path or IO layer
