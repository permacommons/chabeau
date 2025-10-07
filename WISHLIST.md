# Chabeau Wishlist

This file contains features and improvements that would be nice to have in future versions of Chabeau.

Items are removed when completed.

## Features

- Custom styling for system messages — [OPEN]
  - Add dedicated `RoleKind::System` with configurable styling separate from assistant messages
  - Allow theme customization of system message colors, prefixes, and formatting
  - Currently system messages use assistant styling which may not be ideal for all themes
- Tiny copy/paste affordances — [PARTIAL]
- Better handling of repeating messages like "Generating..."
  - Deduplicate/compress repeated status lines — [OPEN]
- In-TUI default picker — [OPEN]
- Tab completion for commands — [OPEN]
- Support common "character cards" — [PARTIAL]
  - Add "Show full description" feature for character picker — [OPEN]
    - Consider adding a detail view (e.g., press 'i' for info) to show full multi-line descriptions
  - Message response presets (useful beyond character cards) [OPEN]
  - Personas [OPEN]
  - Prompt substitutions [OPEN]
- Make assistant messages editable (may require further rethinking of input area) — [OPEN]
- Basic "push a file into context" support — [OPEN]
- "Rapid refine" - apply a previously created prompt to an output — [OPEN]
- Microphone/speaker support? — [OPEN]
- Extend span metadata to code blocks (e.g., `SpanKind::CodeBlock`) to unlock richer TUI interactions — [OPEN]

## Code quality

### High priority

- Logging durability — [OPEN]
  - Make log rewrites (after truncate/in-place edit) atomic via temp file + rename — [OPEN]
  - Optionally append a log marker indicating manual history edits — [OPEN]
- Tests — [OPEN]
  - Add integration-style tests for event handling if feasible (simulate key events) — [OPEN]
  - Consider adding integration tests for the complete Del key workflow in picker dialogs — [OPEN]
  - Consider testing UI state changes after Del key operations (picker refresh) — [OPEN]
### Medium priority

- Reduce duplication in `src/ui/markdown.rs` for code block flushing (extract helper) — [OPEN]
- Consolidate plain vs markdown rendering path selection — [PARTIAL]
- Height/scroll DRYing — [PARTIAL]
  - Continue standardizing on the conversation controller helpers across renderer and chat loop — [OPEN]
- Stream spawning DRYing — [PARTIAL]
  - Use a config struct (already added) and consider moving the helper out of `chat_loop` for reuse — [PARTIAL]
- Unify command routing in `src/commands/mod.rs` — [OPEN]
  - Replace the long conditional ladder with a registry of commands plus shared helpers so new commands stay declarative and testable — [OPEN]

### Low priority

- Centralize help text — [OPEN]
  - Create a small `ui/help.rs` with canonical key-hint strings used by CLI long_about, `/help`, and renderer titles — [OPEN]
- Picker OSC8 state handling — [OPEN]
  - Investigate replacing the temporary clone used to strip link modifiers with a render-time style mask so we reuse the cached transcript buffer and toggle hyperlink styling without allocating new `Line` vectors when pickers open/close.