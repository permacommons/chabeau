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

1. Height/scroll DRYing across renderer and chat loop [PARTIAL]
- Rationale:
  - Impact: Eliminates duplicated height/scroll math, reducing off-by-one errors and stabilizing navigation across views.
  - Effort: Moderate. Centralizing calculation and reusing helpers requires coordinated refactors but limited surface changes.
  - Risk: Moderate. Behavior shifts in clamping and scroll positioning; mitigate with targeted tests around boundary cases.
- Actions:
  - Standardize available-height computation by delegating renderer logic currently at [ui()](src/ui/renderer.rs:71), [ui()](src/ui/renderer.rs:75), [ui()](src/ui/renderer.rs:81) to [ConversationController.calculate_available_height()](src/core/app/conversation.rs:307), removing local calculations.
  - Replace ad hoc scroll math with helpers across layers, preferring [ConversationController.update_scroll_position()](src/core/app/conversation.rs:272), [ConversationController.scroll_index_into_view()](src/core/app/conversation.rs:300), and [ScrollCalculator.scroll_offset_to_line_start()](src/utils/scroll.rs:493) plus [ScrollCalculator.calculate_wrapped_line_count()](src/utils/scroll.rs:568).
  - Continue standardizing on the conversation controller helpers across renderer and chat loop [OPEN].
- References:
  - [ui()](src/ui/renderer.rs:71), [ui()](src/ui/renderer.rs:75), [ui()](src/ui/renderer.rs:81)
  - [ConversationController.update_scroll_position()](src/core/app/conversation.rs:272)
  - [ConversationController.calculate_scroll_to_message()](src/core/app/conversation.rs:283)
  - [ConversationController.scroll_index_into_view()](src/core/app/conversation.rs:300)
  - [ConversationController.calculate_available_height()](src/core/app/conversation.rs:307)
  - [ScrollCalculator.scroll_offset_to_line_start()](src/utils/scroll.rs:493), [ScrollCalculator.calculate_wrapped_line_count()](src/utils/scroll.rs:568), [ScrollCalculator.calculate_scroll_to_message_with_flags()](src/utils/scroll.rs:667)

2. Consolidate plain vs markdown rendering path selection [PARTIAL]
- Rationale:
  - Impact: Reduces branching and drift between plain/markdown paths; improves correctness for wrapping, tables, and scrolling.
  - Effort: Moderate. Centralizing selection and routing through a single entry point impacts renderer and scroll build paths.
  - Risk: Low–Moderate. Some code paths become dead; ensure feature parity with wrap, table, and highlight behavior.
- Actions:
  - Enforce a single entry point by routing all message rendering through [render_message_with_config()](src/ui/markdown.rs:164) and centralizing mode selection in [MessageRenderConfig.markdown()](src/ui/markdown.rs:105).
  - Collapse renderer branching by checking the unified config once inside [ui()](src/ui/renderer.rs:33), [ui()](src/ui/renderer.rs:42), [ui()](src/ui/renderer.rs:55), delegating downstream behavior.
  - Align scroll-build paths to share flags and width logic, preferring the common-with-flags signatures: [ScrollCalculator.build_layout_with_theme_and_flags_and_width()](src/utils/scroll.rs:326), [ScrollCalculator.build_display_lines_up_to_with_flags_and_width()](src/utils/scroll.rs:530) and plain-path handling at [ScrollCalculator.build_display_lines_up_to_with_flags_and_width()](src/utils/scroll.rs:553).
- References:
  - [render_message_with_config()](src/ui/markdown.rs:164), [MessageRenderConfig.markdown()](src/ui/markdown.rs:105)
  - [ui()](src/ui/renderer.rs:33), [ui()](src/ui/renderer.rs:42), [ui()](src/ui/renderer.rs:55)
  - [ScrollCalculator.build_layout_with_theme_and_flags_and_width()](src/utils/scroll.rs:326)
  - [ScrollCalculator.build_layout_with_theme_and_selection_and_flags_and_width()](src/utils/scroll.rs:370)
  - [ScrollCalculator.build_layout_with_codeblock_highlight_and_flags_and_width()](src/utils/scroll.rs:453)
  - [ScrollCalculator.build_display_lines_up_to_with_flags_and_width()](src/utils/scroll.rs:530), [ScrollCalculator.build_display_lines_up_to_with_flags_and_width()](src/utils/scroll.rs:553)
  - [wrap_spans_to_width_generic_shared()](src/ui/markdown_wrap.rs:9), [TableRenderer](src/ui/markdown/table.rs:21), [highlight_code_block()](src/utils/syntax.rs:139)

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