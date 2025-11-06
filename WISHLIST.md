# Chabeau Wishlist

This file contains features and improvements that would be nice to have in future versions of Chabeau.

Items are removed when completed.

## Features

- Better handling of repeating messages like "Generating..."
  - Deduplicate/compress repeated status lines — [OPEN]
- Support common "character cards" — [PARTIAL]
  - Lorebook/world info support from character cards — [OPEN]
- Basic "push a file into context" support — [OPEN]
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


3. Reduce duplication in src/ui/markdown.rs for code block flushing [OPEN]
- Rationale:
  - Impact: Single source of truth for code-block termination prevents subtle newline/trailing span inconsistencies and highlight glitches.
  - Effort: Low–Moderate. Replace multiple flush points and route through a shared function.
  - Risk: Low. Localized change; verify against known codeblock edge cases.
- Actions:
  - Route all code-block terminations through [flush_code_block_buffer()](src/ui/markdown.rs:2661), pruning local flush logic in [Tag::CodeBlock](src/ui/markdown.rs:401), [TagEnd::CodeBlock](src/ui/markdown.rs:504), and [TagEnd::CodeBlock](src/ui/markdown.rs:518).
  - Align readers/builders to consume unified flush semantics by reviewing range/content derivations in [compute_codeblock_ranges()](src/ui/markdown.rs:759), [compute_codeblock_ranges_with_width_and_policy()](src/ui/markdown.rs:829), and [compute_codeblock_contents_with_lang()](src/ui/markdown.rs:884).
- References:
  - [Tag::CodeBlock](src/ui/markdown.rs:401)
  - [TagEnd::CodeBlock](src/ui/markdown.rs:504), [TagEnd::CodeBlock](src/ui/markdown.rs:518)
  - [flush_code_block_buffer()](src/ui/markdown.rs:2661)
  - [compute_codeblock_ranges()](src/ui/markdown.rs:759)
  - [compute_codeblock_ranges_with_width_and_policy()](src/ui/markdown.rs:829)
  - [compute_codeblock_contents_with_lang()](src/ui/markdown.rs:884)

### Low priority

- Centralize help text — [OPEN]
  - Create a small `ui/help.rs` with canonical key-hint strings used by CLI long_about, `/help`, and renderer titles — [OPEN]
- Picker OSC8 state handling — [OPEN]
  - Investigate replacing the temporary clone used to strip link modifiers with a render-time style mask so we reuse the cached transcript buffer and toggle hyperlink styling without allocating new `Line` vectors when pickers open/close.