# Chabeau Wishlist

This file contains features and improvements that would be nice to have in future versions of Chabeau.

Items are removed when completed.

## Features

- More complete keyboard mapping (Page Up/Dn; Ctrl+D) — [OPEN]
- Tiny copy/paste affordances — [PARTIAL]
- Better handling of repeating messages like "Generating..."
  - Deduplicate/compress repeated status lines — [OPEN]
- In-TUI provider selector — [OPEN]
- In-TUI default picker — [OPEN]
- Tab completion for commands — [OPEN]
- Support common "character cards" — [OPEN]
- Rapid initialization of "modes" like "Concise command help" or "Act as a reviewer for" — [OPEN]
- Make assistant messages editable (may require further rethinking of input area) — [OPEN]
- Basic "push a file into context" support — [OPEN]
- "Rapid refine" - apply a previously created prompt to an output — [OPEN]
- Microphone/speaker support? — [OPEN]

## Code quality

- Refactor `src/ui/chat_loop.rs` into smaller, testable units — [OPEN]
  - Extract input handling, selection mode, picker handling, and streaming dispatch into helpers/modules — [OPEN]
  - Add pure functions for selection navigation (wrap-around) and reuse across keys (↑/↓/j/k/Ctrl+P) — [OPEN]
- Reduce duplication in `src/ui/markdown.rs` for code block flushing (extract helper) — [OPEN]
- Consolidate plain vs markdown rendering path selection — [PARTIAL]
- Centralize help text — [OPEN]
  - Create a small `ui/help.rs` with canonical key-hint strings used by CLI long_about, `/help`, and renderer titles — [OPEN]
- Logging durability — [OPEN]
  - Make log rewrites (after truncate/in-place edit) atomic via temp file + rename — [OPEN]
  - Optionally append a log marker indicating manual history edits — [OPEN]
- Height/scroll DRYing — [PARTIAL]
  - Prefer `App::calculate_available_height` everywhere; keep a single helper for “scroll selected index into view” (already added) — [PARTIAL]
  - Standardize usage in renderer and chat loop — [OPEN]
- Stream spawning DRYing — [PARTIAL]
  - Use a config struct (already added) and consider moving the helper out of `chat_loop` for reuse — [PARTIAL]
- Tests — [OPEN]
  - Add unit tests for selection wrap-around and mode transitions (pure helpers) — [OPEN]
  - Add integration-style tests for event handling if feasible (simulate key events) — [OPEN]
  - Add tests for `/markdown` and `/syntax` commands by injecting a test config path or IO layer — [OPEN]
  - Consider adding integration tests for the complete Del key workflow in picker dialogs — [OPEN]
  - Consider testing UI state changes after Del key operations (picker refresh) — [OPEN]
