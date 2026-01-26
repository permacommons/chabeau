# Chabeau Wishlist

This file contains features and improvements that would be nice to have in future versions of Chabeau.

Items are removed when completed.

## Features

- Better handling of repeating messages like "Generating..."
  - Deduplicate/compress repeated status lines — [OPEN]
- Lorebook/world info support for character cards — [OPEN]
- Basic "push a file into context" support — [OPEN]
- Microphone/speaker support? — [OPEN]
- MCP: handle notifications (listChanged, progress, logging) for streamable HTTP and stdio — [OPEN]
- MCP: support long-lived SSE response streams within streamable HTTP — [OPEN]
- MCP: expand capabilities negotiation (client capabilities + server capabilities tracking) — [OPEN]
- MCP: add optional features (sampling/logging/roots/resource subscriptions) as needed — [OPEN]

## Code quality

### High priority

- Tests — [OPEN]
  - Add integration-style tests for event handling if feasible (simulate key events) — [OPEN]
  - Consider adding integration tests for the complete Del key workflow in picker dialogs — [OPEN]
  - Consider testing UI state changes after Del key operations (picker refresh) — [OPEN]

### Low priority

- Centralize help text — [OPEN]
  - Create a small `ui/help.rs` with canonical key-hint strings used by CLI long_about, `/help`, and renderer titles — [OPEN]
- Picker OSC8 state handling — [OPEN]
  - Investigate replacing the temporary clone used to strip link modifiers with a render-time style mask so we reuse the cached transcript buffer and toggle hyperlink styling without allocating new `Line` vectors when pickers open/close.
