# Chabeau Architecture (reference: d3ebd0c)

> This document is aligned to commit `d3ebd0c` on the main codebase. If you're
> reading a newer checkout, prefer this file as a map and then verify details in
> the referenced modules.

## System overview
Chabeau is a terminal-first chat client that wraps OpenAI-compatible APIs behind a
Ratatui interface. The runtime combines:

- Tokio for async orchestration.
- Reqwest for provider and MCP HTTP traffic.
- Ratatui + Crossterm for terminal UI/event handling.
- `rust-mcp-schema` types for MCP request/response modeling.

Primary crate roots:

- `src/main.rs` – process entrypoint.
- `src/cli/mod.rs` – argument parsing and command dispatch.
- `src/ui/` – interactive chat loop and rendering.
- `src/core/` – app state, streaming orchestration, config, auth.
- `src/mcp/` – MCP client manager, transport adapters, registry/permissions.

## Runtime entry points and CLI flow
Execution starts in `src/main.rs`, then calls into `cli::run()`.

In `src/cli/mod.rs`, clap-parsed subcommands route to non-UI flows (`auth`,
`deauth`, `set`, `say`, MCP token ops) or into the interactive chat path. Global
flags such as `--debug-mcp` and `--disable-mcp` are resolved here and threaded
forward so behavior is consistent in both one-shot and interactive execution.

Shared prompt/input behavior for CLI setup flows lives in
`src/utils/line_editor.rs`.

## Chat session bootstrap
Interactive chat setup is handled by `src/ui/chat_loop/setup.rs`.
`bootstrap_app(...)` loads configuration, resolves auth through `AuthManager`,
builds `App`, and chooses whether to launch provider/model pickers based on the
current config state.

## Core application state
The central runtime object is `App` (`src/core/app/mod.rs`), with session details
kept in `SessionContext` (`src/core/app/session.rs`).

Notable state domains:

- Conversation/history and UI mode/input focus.
- Picker state (model/theme/provider/character/persona/preset/MCP prompt).
- Streaming lifecycle and pending tool calls.
- MCP manager, server enablement, and per-tool approval memory.
- Tool inspection overlay state (`src/core/app/inspect.rs`).

## MCP configuration and authentication
MCP server configuration is defined in `src/core/config/data.rs`
(`McpServerConfig`). It includes transport mode, URLs/commands, env/args,
optional headers, tool allow-lists, and payload retention policy.

HTTP auth tokens are stored via `McpTokenStore` in `src/core/mcp_auth.rs`.

## MCP client subsystem
The MCP client implementation is now organized under `src/mcp/client/`:

- `src/mcp/client/mod.rs` – `McpClientManager`, per-server runtime state, and
  connect/refresh orchestration.
- `src/mcp/client/operations.rs` – protocol-level operations such as
  `execute_tool_call`, `execute_resource_read`, `execute_prompt`, and helpers for
  client result/error responses.
- `src/mcp/client/protocol.rs` – request/response parsing/normalization helpers.
- `src/mcp/client/transport_http.rs` and `src/mcp/client/transport_stdio.rs` –
  transport-specific client behavior.
- `src/mcp/client/tests.rs` – focused manager/transport tests.

Transport primitives are shared through `src/mcp/transport/`:

- `streamable_http.rs`
- `stdio.rs`
- `mod.rs`

Server filtering and permission memory live in:

- `src/mcp/registry.rs`
- `src/mcp/permissions.rs`

## Tool-calling and MCP execution pipeline
MCP tool/resource context is injected when building requests in
`src/core/app/streaming.rs`.

Runtime tool execution and follow-up flow are dispatched from
`src/core/app/actions/streaming.rs`, with focused logic split across:

- `src/core/app/actions/mcp_gate.rs`
- `src/core/app/actions/tool_calls.rs`
- `src/core/app/actions/sampling.rs`
- `src/core/app/actions/stream_lifecycle.rs`
- `src/core/app/actions/stream_errors.rs`

These reducers coordinate permission prompts, MCP calls, sampling handshakes, and
transcript summary updates.

## MCP slash commands and prompt invocation
Slash command routing is defined in `src/commands/mod.rs` with MCP handlers
under `src/commands/handlers/mcp.rs` and prompt parsing in
`src/commands/mcp_prompt_parser.rs`.

The chat loop schedules MCP refresh/call work via executor helpers in
`src/ui/chat_loop/executors/` (`mcp_init.rs`, `mcp_tools.rs`).

## UI loop and action system
The interactive event loop is centered in `src/ui/chat_loop/event_loop.rs` and
re-exported through `src/ui/chat_loop/mod.rs` as `run_chat(...)`.

Supporting modules:

- `src/ui/chat_loop/keybindings/` – mode-aware key routing.
- `src/ui/chat_loop/modes.rs` – mode definitions.
- `src/ui/chat_loop/lifecycle.rs` – terminal setup/restore and cursor styling.
- `src/ui/chat_loop/executors/` – background task spawners for model loading,
  MCP init, tool/prompt execution, and sampling callbacks.

`AppActionDispatcher` and reducers live in `src/core/app/actions/` and translate
UI intents into state mutation plus deferred `AppCommand`s.

## Tool inspection and decode workflow
Tool inspection UI is managed by `InspectController`
(`src/core/app/inspect.rs`) and input handlers in
`src/core/app/actions/input/inspect.rs`. Rendering for inspection overlays lives
in `src/ui/renderer.rs`.

## Characters, personas, and presets
Character loading/caching is implemented in `src/character/service.rs` and
related modules under `src/character/`.

Persona and preset management live in:

- `src/core/persona.rs`
- `src/core/preset.rs`

These are initialized into `App` so users can swap behavior templates without
reloading base config.

## Streaming pipeline
Provider streaming transport is implemented in `src/core/chat_stream.rs`.
`ChatStreamService` converts provider events into internal stream messages,
normalizes malformed chunks, and propagates cancellation/error signaling back to
the UI loop.

## UI rendering and performance safeguards
The primary renderer is `src/ui/renderer.rs`.

`src/ui/chat_loop/event_loop.rs` dynamically adjusts polling/sleep behavior based
on activity (typing, animation, idle) to reduce idle CPU usage while preserving
interactive responsiveness.

## Configuration orchestration and test isolation
Config reads/writes are serialized through `ConfigOrchestrator`
(`src/core/config/orchestrator.rs`).

Test helpers in `src/utils/test_utils.rs` can redirect config persistence to a
temporary XDG config root so test runs do not mutate user config.

## Test layout conventions
Larger test suites are split into sibling files to keep runtime modules focused
while still exercising private behavior via `use super::*`.

Examples:

- `src/commands/tests.rs`
- `src/cli/tests.rs`
- `src/ui/chat_loop/event_loop_tests.rs`
- `src/mcp/client/tests.rs`
