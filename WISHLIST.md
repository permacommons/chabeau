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
- Support common "character cards" — [OPEN]
- Rapid initialization of "modes" like "Concise command help" or "Act as a reviewer for" — [OPEN]
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

## Architecture

- Incremental chat loop action system — [OPEN]
  - **Motivation:** Reduce pervasive `Arc<Mutex<App>>` usage in the TUI loop to cut contention, simplify key handlers, and make UI state changes easier to reason about and test.
  - **Step 1:** Introduce an internal `AppAction` enum plus dispatcher, then migrate the stream pipeline to emit actions instead of locking the app directly. — [DONE]
  - **Step 2:** Refactor `run_chat` so it owns `App` and drains the action queue, keeping existing handlers temporarily by translating their direct mutations into actions inside the loop. — [DONE]
  - **Step 3:** Convert keybinding handlers in batches (basic controls → navigation/editing → picker modes) to emit actions, adding focused regression tests for each batch. — [DONE]
  - **Step 4:** Migrate remaining async helpers (external editor, retry, picker flows) to use the dispatcher, cleaning up now-unnecessary `Arc<Mutex<_>>` plumbing. — [DONE]
  - **Step 5:** Remove legacy locking helpers and tighten the event-loop API in incremental slices:
    - **5a:** Move picker presentation mutations behind dedicated actions (open/close, preview, selection persistence). Ensure `/provider`, `/model`, and `/theme` flows drive the dispatcher exclusively, add focused tests for preview rollbacks, and route loader-triggered redraws through queued actions.
    - **5b:** Translate command submission, file-prompt, and in-place edit workflows into action producers so command handling stops mutating shared state. Cover happy/error paths (dump, save-block, retries) with regression tests and ensure logging side effects remain intact.
    - **5c:** Drop the deprecated locking helpers from `chat_loop`, document the single-owner dispatcher model, and audit public APIs (especially `ChatStreamService`, picker controllers, and commands) for the new invariants before promoting the pattern in README/architecture notes.
  - **Step 6:** Post-migration cleanup — [OPEN]
    - Sweep for remaining call sites that take `Arc<Mutex<App>>` solely for mutation side effects and convert to read-only access or action emission.
    - Prune or rewrite legacy tests that still exercise the deprecated locking helpers; ensure action-centric tests cover enter, picker, and retry flows.
    - Remove helper functions that became redundant after the dispatcher rollout (e.g., old picker refresh utilities) and modernize docs/examples to show the action-first patterns.
