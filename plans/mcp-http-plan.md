# MCP-over-HTTP integration plan (Bearer tokens)

Goal: Add MCP-over-HTTP client support with automatic tool/resource usage, a per-tool permission prompt (Allow once / Allow for session / Deny) in the input area, and a `/mcp` command that lists registered MCP servers and their tools/resources/prompts. Tokens live in keyring; non-secret MCP server config lives in `config.toml`.

## 1. Confirm MCP transport + SDK wiring
- Use `rust-mcp-sdk` client transports: Streamable HTTP and SSE from day one; explicitly skip stdio transport.
- Pin MCP protocol version to the SDK latest supported version and accept servers that negotiate to that or older compatible versions.
- Auto-use MCP tools and resources; expose MCP prompts as manual-only.
- Implement proper tool-calling support end-to-end (tool schemas in requests, tool-call parsing, tool-result messages) as a prerequisite for MCP integration.
  - Treat tool calls/results as their own message kind in the transcript, similar to app messages.
Notes:
- Added `rust-mcp-sdk` dependency and wired streamable HTTP + SSE client transport basics.
- Tool-calling support and message-type wiring are still pending.
 - Added tool call/result transcript roles and rendering scaffolding (no tool execution yet).
 - Streaming tool-call deltas now aggregate into per-stream pending tool calls and flush into the transcript on completion.
 - MCP initialization now runs in the background on startup; first send can wait for tools without blocking typing.
- Added a built-in MCP tool-use preamble and inject it into the system prompt when tools are included.

## 2. Configuration + keyring storage
- Extend `src/core/config/data.rs` with MCP server entries (id, display_name, base_url, transport, allowed_tools list optional, protocol_version optional, enabled flag).
- Add config loading/saving in `src/core/config` flows, plus sample in `examples/config.toml.sample`.
- Add keyring storage in `src/auth`-adjacent module (or new `src/core/mcp_auth.rs`) mirroring provider token flows:
  - `mcp auth` is not required; tokens are stored when config references a server and user runs a new CLI action (see below).
  - Add functions: get_token(server_id), set_token(server_id), remove_token(server_id).
- Update `chabeau auth` UI? Probably not; for now use `chabeau mcp token <server>` or prompt on first use (see below). If we must avoid new CLI, add a prompt flow on first MCP connection to request token and store in keyring.
Notes:
- Implemented `McpServerConfig` in `src/core/config/data.rs` with list helpers and enabled helper.
- Added `mcp_servers` to `Config` and updated `examples/config.toml.sample`.
- Added `src/core/mcp_auth.rs` with keyring-backed token get/set/remove.
- Added `chabeau mcp token <server>` CLI command to prompt for and store bearer tokens.

## 3. MCP client subsystem
- Create `src/mcp/` module:
  - `client.rs`: wraps `rust-mcp-sdk` client runtime; handles initialization, connection lifecycle, and cached server info.
  - `registry.rs`: loads config, resolves enabled servers, caches tool/resource/prompt lists, and provides lookup by server/tool id.
  - `permissions.rs`: in-memory session decision store keyed by (server_id, tool_name) -> {allow_once, allow_session, deny}.
- Decide async runtime integration: reuse existing Tokio runtime in UI; ensure MCP client uses shared runtime and is shut down on app exit.
Notes:
- Added `src/mcp/registry.rs` (config-based enabled servers only) and `src/mcp/permissions.rs` (session decision store).
- Added `src/mcp/client.rs` to connect via `rust-mcp-sdk` (streamable HTTP + SSE) and cache tools/resources/prompts.
- Streamable HTTP now avoids the standalone SSE stream to prevent HTTP GETs against non-SSE servers.
- MCP connections no longer run during app startup; `/mcp` triggers on-demand connections.
- Added `chabeau --debug-mcp` to enable verbose MCP tracing for transport debugging.
- Tool/resource/prompt listings now refresh only when explicitly requested via `/mcp tools|resources|prompts`.
- Restored explicit Accept/Content-Type headers in MCP requests to keep Express JSON parsing consistent.
- MCP connections now reuse existing initialized clients to preserve session ids.
- Streamable HTTP uses direct HTTP requests for session bootstrap/listing to avoid SSE hangs and capture session ids.
- `/mcp` debug output now includes session id when `--debug-mcp` is set.
- Connection status and cached listings now live in `App.mcp`; shutdown wiring is still pending.

## 4. Permission prompt UX in input area
- Add a new input prompt mode similar to file save prompts:
  - Title: "Allow tool?" with tool name + server display name.
  - Buttons/choices: Allow once / Allow for session / Deny.
- Ensure it blocks tool execution until a selection is made.
- Wire to existing input prompt handlers in `src/ui/chat_loop` and `src/core/app/actions`:
  - New AppAction: `PromptToolPermission { server_id, tool_name, args_summary, request_id }`.
  - New AppCommand: `RunMcpTool { ... }` after approval, else append a denial app message.
Notes:
- Added `UiMode::ToolPrompt` + title-bar prompt with compact A/S/D/Esc choices and args summary.
- Added ToolPrompt keybindings handler (A/S/D/Enter/Esc) and Ctrl+C support for emergency exit.
- Wired ToolPermissionDecision handling to record session decisions and gate tool execution.
- Added tool-run status line while MCP tool executes.

## 5. Automatic MCP tool invocation pipeline
- Implement tool-calling support as the primary path (manual `/mcp call` is optional for debugging):
  - When LLM response indicates a tool call, check MCP registry for tool name matches (per server), then request permission and execute.
  - Encode tool calls/results using a dedicated message type (not plain app messages) to preserve structure in the transcript.
- Execute tool call via SDK client; convert results to chat messages:
  - Render tool output as tool message blocks in the transcript.
  - Errors become `AppMessageKind::Error` with context.
Notes:
- MCP tool schemas are injected into chat requests when cached tool listings are available.
- Tool call deltas now queue structured tool requests with assistant tool-call records.
- Added tool permission gate + MCP execution path (SSE via SDK, streamable HTTP via direct HTTP).
- Tool results are appended to transcript and serialized into tool role messages for the follow-up model call.
- Follow-up stream now includes assistant tool_calls + tool results using cached `last_stream_api_messages`.
- Added MCP tool-call unsupported detection: on provider/tool error we disable MCP tools, show a warning, retry the failed message without tools, and re-enable on model change.

## 6. MCP resources + prompts exposure
- Resources: auto-use by MCP tool calls only (no direct UI for now). If auto-use is ambiguous, skip until needed.
- Prompts: expose manual usage only. Add slash command or `/mcp` list output for prompts; maybe `/prompt mcp:<server>/<prompt>` in future.
Notes:
- Added `/server-id:prompt-id` MCP prompt invocation with key=value args and interactive prompting for missing required args.
- Prompt results are inserted into the transcript and immediately sent to the model for a response.
- Slash command autocomplete now includes MCP prompt commands when prompt listings are cached.
- Added `mcp_read_resource` tool schema when resources are cached, and inject a resources list into the system prompt.
- Resource reads are routed through MCP `resources/read` and returned as tool results.

## 7. `/mcp` command implementation
- Add to `src/commands/mod.rs`:
  - `/mcp` with no args: list registered MCP servers + status (enabled, connected, token present).
  - `/mcp tools <server>` list tools.
  - `/mcp resources <server>` list resources.
  - `/mcp prompts <server>` list prompts.
- Output should be Markdown with sections for readability.
Notes:
- Implemented `/mcp` command and registry entry with Markdown output.
- `/mcp` now connects on demand and uses `App.mcp` connection state + cached lists (tools/resources/prompts show real data when available).
- Token status checks use keyring when enabled; connected status now reports yes/no and last error if present.
- Added `chabeau mcp token <server>` for entering tokens.

## 8. Tests
- Unit tests for:
  - Config load/save of MCP servers.
  - Permission decision store (session lifetime, allow once vs session).
  - `/mcp` command output formatting for empty vs populated lists.
- Integration tests (if feasible) with mock MCP server using rust-mcp-sdk test helpers.
Notes:
- Added config persistence test for MCP servers.
- Added permission store unit tests.
- Added `/mcp` command tests for empty config and allowed tools output.
- No integration tests yet.

## 9. README updates
- Add MCP section: how to configure MCP servers, token storage in keyring, auto tool permissions, and `/mcp` listing.
- No bugfix notes.
Notes:
- Added README MCP section covering configuration and `/mcp` command; clarified that live listings require MCP client wiring.
- Documented `chabeau mcp token <server>` in the MCP section.
- Did not document auto tool permissions since permission UX is not implemented yet.
 - Updated MCP section to note on-demand `/mcp` connections and pending tool-calling permissions.
 - Added `--debug-mcp` mention for MCP transport debugging.
- Updated MCP section to describe tool permission prompts and tool execution flow.

## 10. Final checks
- Run `cargo fmt`, `cargo check`, `cargo test`, `cargo clippy`.
- Consider benchmarks only if performance-sensitive changes land in rendering or streaming paths.
- Optionally suggest WISHLIST.md changes after completion.
Notes:
- Ran `cargo fmt`, `cargo check`, `cargo clippy`.
- `cargo test` not run in this pass (per user request to ask first).
 - Ran `cargo fmt` for this phase; `cargo check`, `cargo test`, and `cargo clippy` pending per user request.
 - Ran `cargo fmt`, `cargo check`, `cargo test`, and `cargo clippy` for the MCP preamble update.
 - Ran `cargo fmt`, `cargo check`, `cargo test`, and `cargo clippy` after MCP tool-support detection changes.
