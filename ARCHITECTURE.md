# Chabeau Architecture (commit 41e7ee64efbbab7e9be7deb9a81f18002dd7ea3e)

## System overview
Chabeau is a terminal-first chat client that wraps OpenAI-compatible APIs behind a rich TUI. The
crate is organised as a collection of focused modules that sit on top of Tokio for async orchestration,
Reqwest for HTTP, and Ratatui for rendering. These dependencies are wired in via the crate manifest and
kept minimal to ease distribution across platforms.【F:Cargo.toml†L1-L55】

## Runtime entry points and CLI flow
Execution begins in `src/main.rs`, which delegates immediately to the CLI module and exits with a non-zero
status on failure so that shell scripting remains predictable.【F:src/main.rs†L1-L12】 The CLI builds a Tokio
runtime, parses arguments with Clap, and routes subcommands such as `auth`, `deauth`, `set`, and `say`
without touching the UI stack. Non-command invocations fall through to `handle_args`, which prepares a
`CharacterService` and either performs management operations or launches the chat loop via `run_chat`.【F:src/cli/mod.rs†L201-L415】【F:src/cli/mod.rs†L287-L329】

## Chat session bootstrap
When a chat session is requested the UI bootstrapper resolves configuration and credentials. It loads the
user's config, consults the keyring-aware `AuthManager`, determines whether a provider picker must be shown,
and prepares a fully initialised `App` when credentials are available. The bootstrapper also honours
`--env` flows, surfaces provider resolution failures, and triggers model pickers when the selected provider
lacks a default model.【F:src/ui/chat_loop/setup.rs†L19-L185】

## Core application state
The heart of the runtime is the `App` struct, which packages the current session, UI state, pickers, shared
character service, persona manager, preset manager, and configuration snapshot. This single owner makes it
easy to mutate session state atomically within the async chat loop.【F:src/core/app/mod.rs†L53-L208】 Session
metadata is tracked in `SessionContext`, including the active provider, HTTP client, logging sink, streaming
bookkeeping, refine settings, and any loaded character greeting state so that downstream components can act
without re-querying configuration.【F:src/core/app/session.rs†L23-L44】 During authenticated startup the app
also activates personas and presets, falling back to provider/model defaults when no CLI override is present
and recording any loading failures directly into the conversation transcript for visibility.【F:src/core/app/mod.rs†L81-L163】

## UI loop and action system
The Ratatui event loop wraps the shared `App` inside an `AppHandle` so tasks can borrow it through an async
mutex. Terminal setup, input polling, and resize handling feed `UiEvent`s into the dispatcher, which resolves
mode-aware keymaps and emits high-level `AppAction`s. Actions are grouped by concern—streaming, input
manipulation, picker interaction, and file prompts—and reducers can return `AppCommand`s that request new
background work such as spawning a stream or loading models. Keybindings are managed by a dedicated registry
that routes input events to handlers based on the active UI mode.【F:src/ui/chat_loop/mod.rs†L1-L42】【F:src/ui/chat_loop/event_loop.rs†L1-L200】【F:src/ui/chat_loop/keybindings/mod.rs†L1-L300】【F:src/core/app/actions/mod.rs†L1-L194】

## Streaming pipeline
Outgoing requests are encapsulated in `StreamParams` and executed by `ChatStreamService`. Each stream runs in
a Tokio task that posts SSE frames into an unbounded channel, normalises malformed input, reports API errors
with helpful Markdown summaries, and honours cancellation tokens so that user interrupts stop work promptly.【F:src/core/chat_stream.rs†L9-L349】

## Configuration, providers, and authentication
Persistent settings live in `core::config`, where TOML-backed structs capture defaults for providers, models,
characters, personas, presets, and text refinement behaviour. These helpers also expose ergonomic display
strings for config paths.【F:src/core/config/data.rs†L6-L187】 Provider resolution is mediated by the
`AuthManager`, which merges built-in metadata, user-defined providers, and keyring lookups while allowing
pure environment-based sessions as a fallback. Rich error types communicate next steps back to the CLI and
bootstrapper.【F:src/auth/mod.rs†L1-L190】【F:src/core/providers.rs†L6-L200】

## Characters, personas, and presets
Character cards are cached and resolved by `CharacterService`, which invalidates stale entries when cache keys
change, supports direct path loads, and falls back to metadata scans when necessary. It also cooperates with
session bootstrapping to honour per-provider defaults.【F:src/character/service.rs†L49-L189】 Personas and
presets are managed by dedicated managers loaded during app construction so that the UI can swap identities
or prompt templates without reloading config from disk.【F:src/core/app/mod.rs†L81-L148】【F:src/core/persona.rs†L1-L200】【F:src/core/preset.rs†L1-L200】

## Commands and input routing
Slash commands share a central registry that distinguishes between messages and control commands. Handlers can
return `CommandResult` variants instructing the UI to continue, pass the text to the model, toggle UI features, trigger message refinement, or open pickers
for themes, providers, models, characters, personas, and presets. File and logging commands reuse shared
controllers so that interactive flows stay consistent across keyboard shortcuts and slash commands.【F:src/commands/mod.rs†L4-L200】

## UI rendering
Rendering is handled by `ui::renderer`, which composes the chat transcript, pickers, and input area using
Ratatui layout primitives. It caches wrapped lines for performance, adapts styling when pickers are open, and
projects mode-specific prompts (e.g., compose, edit, or streaming indicators) into the title bar. Scroll state
and OSC hyperlink metadata are recomputed only when necessary to keep redraws responsive.【F:src/ui/renderer.rs†L17-L200】

