# Plan: Assistant message editing (Ctrl+X)

## Goals
- Allow selecting assistant messages via Ctrl+X, mirroring user edit-select behavior but covering assistant responses.
- Provide editing actions (edit in place, truncate, edit+truncate) without resending, including updated status messaging and focus handling.
- Ensure UI chrome and input prompts reflect assistant editing state.
- Maintain minimal duplication by reusing/generalizing existing edit-select infrastructure.

## Steps
1. **Extend UI state to track selection targets and assistant-edit context**
   - Update `UiMode::EditSelect` in `src/core/app/ui_state.rs` to store which role is being selected (user vs assistant) via a new enum.
   - Add helper methods for locating previous/next/first/last message indexes by role, exposing convenience wrappers for user and assistant.
   - Introduce a flag/method for marking when the input buffer is being used to edit an assistant response so the renderer can show "Edit message".
   - Adjust existing edit-select helpers (`enter_*`, getters, setters) and associated unit tests to accommodate the new enum without regressing user flow.

2. **Generalize edit-select mode handling logic**
   - Refactor `handle_edit_select_mode_event` in `src/ui/chat_loop/modes.rs` into a role-aware helper that supports both user and assistant behavior while sharing traversal logic.
   - Ensure assistant-specific actions: set status "No assistant messages" when none exist, prevent resend triggers, update logging/cache, and manage the assistant editing flag when editing via Enter/Del/e.
   - Update or add unit tests covering navigation, truncation, and editing workflows for assistant messages alongside existing user tests.

3. **Add Ctrl+X keybinding and mode integration**
   - Implement a new handler in `src/ui/chat_loop/keybindings/handlers.rs` that invokes the generalized edit-select logic for assistant messages, cycling through assistant responses similar to Ctrl+P.
   - Register the handler in `src/ui/chat_loop/keybindings/mod.rs`, ensure `should_handle_as_text_input` excludes Ctrl+X, and keep Tab focus lock behavior via shared `EditSelect` context.
   - Expand keybinding/event-loop tests (e.g., in `src/ui/chat_loop/event_loop.rs`) to verify Tab remains locked during assistant selection and that entering the mode works as expected.

4. **Update renderer and in-place edit plumbing**
   - Modify `src/ui/renderer.rs` to highlight selected assistant messages in edit-select mode and to show "Edit message" when the assistant editing flag is active while typing.
   - Allow in-place edits (`complete_in_place_edit` in `src/core/app/ui_helpers.rs`) for assistant roles and ensure the flag resets on cancel/clear paths.
   - Cover these UI adjustments with targeted unit tests where feasible (e.g., verifying flag transitions in `ui_state` tests and completing assistant edits).

5. **Documentation and help updates**
   - Document the new shortcut in `README.md` and `src/builtins/help.md`.
   - Update `CHANGELOG.md` (if appropriate) to capture the feature.

6. **Validation**
   - Run `cargo fmt`, `cargo clippy`, `cargo check`, and `cargo test`.
