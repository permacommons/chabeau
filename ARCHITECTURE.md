# Chabeau Architecture (8b47a71)

## System overview
Chabeau is a terminal-first chat client that wraps OpenAI-compatible APIs behind a rich TUI. The
crate is organized as a collection of focused modules that sit on top of Tokio for async orchestration,
Reqwest for HTTP, Ratatui for rendering, and rust-mcp-schema for Model Context Protocol (MCP) payloads
and schema types.【F:Cargo.toml†L1-L55】

## Runtime entry points and CLI flow
Execution begins in `src/main.rs`, which delegates to the CLI module and exits with a non-zero status
on failure so that shell scripting remains predictable.【F:src/main.rs†L1-L12】 The CLI builds a Tokio
runtime, parses arguments with Clap, and routes subcommands such as `auth`, `deauth`, `set`, `say`,
and MCP token management without touching the UI stack.【F:src/cli/mod.rs†L201-L415】【F:src/cli/mod.rs†L726-L781】
Global flags like `--debug-mcp` (trace logging) and `--disable-mcp` (session-wide MCP disable) are
wired into the same entry path, ensuring they apply consistently to both TUI and one-shot `say`
commands.【F:src/cli/mod.rs†L167-L739】【F:src/cli/say.rs†L267-L333】

## Chat session bootstrap
When a chat session is requested the UI bootstrapper resolves configuration and credentials. It loads the
user's config, consults the keyring-aware `AuthManager`, determines whether a provider picker must be shown,
and prepares a fully initialized `App` when credentials are available. The bootstrapper also honors
`--env` flows, surfaces provider resolution failures, and triggers model pickers when the selected provider
lacks a default model.【F:src/ui/chat_loop/setup.rs†L19-L185】

## Core application state
The heart of the runtime is the `App` struct, which packages the current session, UI state, pickers, shared
character service, persona manager, preset manager, configuration snapshot, plus the MCP client manager,
MCP permission store, and inspection controller for tool-call introspection.【F:src/core/app/mod.rs†L53-L208】
Session metadata is tracked in `SessionContext`, including the active provider, HTTP client, logging sink,
streaming bookkeeping, tool-call queues, tool payload history, and MCP enablement flags so downstream
components can act without re-querying configuration.【F:src/core/app/session.rs†L23-L148】

## MCP configuration and authentication
MCP servers are configured in `config.toml` using `McpServerConfig`, which supports HTTP base URLs, stdio
commands, allowed tool lists, protocol overrides, payload retention policies, and per-server YOLO settings
for auto-approval of tool calls.【F:src/core/config/data.rs†L70-L118】 Tokens for HTTP servers are stored in
the system keyring by `McpTokenStore`, mirroring provider auth behavior while keeping secrets out of
config files.【F:src/core/mcp_auth.rs†L1-L63】

## MCP client subsystem
The MCP client layer lives in `src/mcp/`, where `McpClientManager` owns per-server state, cached listings,
and the active transport implementation.【F:src/mcp/client.rs†L1-L168】 Streamable HTTP and stdio transports
are supported; stdio launches child processes and streams JSON-RPC frames over stdin/stdout, while HTTP uses
SSE/streamable responses.【F:src/mcp/client.rs†L18-L209】 A lightweight registry filters enabled servers for
tool discovery, and a permission store tracks per-tool approval decisions for the current session.【F:src/mcp/registry.rs†L1-L31】【F:src/mcp/permissions.rs†L1-L74】
The large `mcp::client` test suite is split into `src/mcp/client/tests.rs` to keep runtime code and tests
separate while still testing private module behavior through `use super::*`.

## Tool-calling and MCP execution pipeline
When building a stream request, Chabeau injects MCP tool schemas, a built-in MCP preamble, and optional
resource/template listings so that the model can call tools and request MCP resources. It also adds a tool
payload retention note; when payloads are summarized, tool summaries include call IDs that can be used with
`chabeau_instant_recall` to pull full outputs back into context.【F:src/core/app/streaming.rs†L9-L214】 Tool calls are queued, permission
prompts are surfaced in the input area, and the execution path supports MCP tools, MCP resource reads,
and MCP sampling (`sampling/createMessage`) requests with separate permission prompts or YOLO auto-approval.
Tool results are summarized into the transcript while raw payload retention is governed by per-server policy
(window/all/turn).【F:src/core/app/actions/streaming.rs†L980-L1611】

## MCP slash commands and prompt invocation
Slash commands in `commands/mod.rs` provide `/mcp` listings, per-server enable/disable toggles, `/yolo`
for MCP auto-approval, and MCP prompt invocation (using `/server-id:prompt-id`) with interactive argument
collection when required.【F:src/commands/mod.rs†L136-L735】 The event loop triggers background refreshes
for MCP listings as commands are issued, keeping cached tools/resources/prompts up to date without blocking
input.【F:src/ui/chat_loop/event_loop.rs†L123-L212】

## UI loop and action system
The Ratatui event loop wraps the shared `App` inside an `AppHandle` so tasks can borrow it through an async
mutex. Terminal setup, input polling, and resize handling feed `UiEvent`s into the dispatcher, which resolves
mode-aware keymaps and emits high-level `AppAction`s. Actions are grouped by concern—streaming, input
manipulation, picker interaction, and file prompts—and reducers can return `AppCommand`s that request new
background work such as spawning a stream or running an MCP tool call.【F:src/ui/chat_loop/mod.rs†L1-L42】【F:src/ui/chat_loop/event_loop.rs†L1-L344】【F:src/core/app/actions/mod.rs†L1-L250】
`AppActionDispatcher` includes typed batch helpers (`dispatch_input_many`, `dispatch_streaming_many`,
`dispatch_picker_many`) so same-domain dispatch paths avoid unnecessary wrapping, while `dispatch_many`
remains the generic entrypoint for intentionally mixed-domain batches.

## Tool inspection and decode workflow
Tool calls and results can be inspected via a full-screen overlay managed by `InspectController`, which
tracks the current tool index, view (request vs result), and a decoded flag for nested JSON display.
Input actions allow users to toggle decoded views and navigate between tool records, while the renderer
adds dedicated inspect chrome and keyboard hints for the overlay.【F:src/core/app/inspect.rs†L1-L155】【F:src/core/app/actions/input/mod.rs†L1-L95】【F:src/core/app/actions/input/inspect.rs†L1-L285】【F:src/ui/renderer.rs†L477-L560】

## Tool result summaries and error labeling
Tool call outcomes are summarized into readable transcript entries that include per-server labels and
argument summaries. Failed tool calls are categorized into “tool error” vs “tool call failure” to make
API issues easier to diagnose and are reflected in both summary strings and status labels.【F:src/core/app/session.rs†L84-L140】【F:src/core/app/actions/streaming.rs†L1445-L1635】

## UI rendering and performance safeguards
Rendering is handled by `ui::renderer`, which composes the chat transcript, pickers, tool prompts, and
inspection overlays using Ratatui layout primitives.【F:src/ui/renderer.rs†L17-L560】 The event loop adapts
its polling interval and inserts idle sleeps when no input or animation is occurring, reducing idle CPU
usage without impacting responsiveness during active sessions.【F:src/ui/chat_loop/event_loop.rs†L1054-L1312】

## Characters, personas, and presets
Character cards are cached and resolved by `CharacterService`, which invalidates stale entries when cache keys
change, supports direct path loads, and falls back to metadata scans when necessary. It also cooperates with
session bootstrapping to honor per-provider defaults.【F:src/character/service.rs†L49-L189】 Personas and
presets are managed by dedicated managers loaded during app construction so that the UI can swap identities
or prompt templates without reloading config from disk.【F:src/core/app/mod.rs†L81-L148】【F:src/core/persona.rs†L1-L200】【F:src/core/preset.rs†L1-L200】

## Commands and input routing
Slash commands share a central registry that distinguishes between messages and control commands. Handlers can
return `CommandResult` variants instructing the UI to continue, pass the text to the model, toggle UI features,
trigger message refinement, or open pickers for themes, providers, models, characters, personas, presets, or
MCP servers/prompts. File and logging commands reuse shared controllers so that interactive flows stay
consistent across keyboard shortcuts and slash commands.【F:src/commands/mod.rs†L4-L735】

## Streaming pipeline
Outgoing requests are encapsulated in `StreamParams` and executed by `ChatStreamService`. Each stream runs in
a Tokio task that posts SSE frames into an unbounded channel, normalizes malformed input, reports API errors
with helpful Markdown summaries, and honors cancellation tokens so that user interrupts stop work promptly.【F:src/core/chat_stream.rs†L9-L349】

## Configuration orchestrator and test isolation
All configuration reads and writes go through `ConfigOrchestrator`, which caches the on-disk state and
serializes mutations so that concurrent access is safe.【F:src/core/config/orchestrator.rs†L13-L80】 The
static `CONFIG_ORCHESTRATOR` points at the real config path; in `#[cfg(test)]` builds a second static,
`TEST_ORCHESTRATOR`, can be swapped in to redirect all `Config::load()`, `Config::mutate()`, and
`Config::save()` calls to an isolated temporary directory.【F:src/core/config/orchestrator.rs†L82-L146】

When `TEST_ORCHESTRATOR` is `None` (the default for most unit tests), the test-mode `Config::mutate()`
applies the mutator to a throwaway `Config::default()` and discards the result without writing to disk.
`Config::load()` does fall through to the real config in this case, but only for reads.

Tests that need round-trip persistence (save then re-load) use `with_test_config_env()`, which creates a
`TempDir`, sets `XDG_CONFIG_HOME` to it, calls `Config::set_test_config_path()` to activate the test
orchestrator, and restores everything on drop.【F:src/utils/test_utils.rs†L28-L106】 This guarantees that
`cargo test` never writes to the user's real `config.toml`.
