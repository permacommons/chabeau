# Changelog

## 0.4.0

### Added
- Compose mode toggled with F4 (Enter inserts newlines; Alt+Enter/Ctrl+J send) to make long-form drafting easier (`src/ui/chat_loop`, `src/builtins/help.md`).
- OSC 8 hyperlink rendering across chat, tables, and pickers via span metadata and a custom Crossterm backend (`src/ui/osc_backend.rs`, `src/ui/osc_state.rs`, `src/ui/osc.rs`).
- Adaptive color fallback that detects truecolor/256/16-color terminals and exposes a `CHABEAU_COLOR` override (`src/utils/color.rs`, `src/core/app.rs`).
- In-app provider picker and config persistence so `/provider` mirrors `/model` for discovery and defaults (`src/ui/picker.rs`, `src/ui/chat_loop`, `src/builtins/help.md`).
- `--env` startup mode to force environment-based credentials when providers are configured (`src/core/app.rs`, `src/cli/mod.rs`).

### Changed
- Introduced a shared width-aware layout engine and span metadata pipeline for Markdown/plain rendering, enabling horizontal table scrolling and consistent wrapping (`src/ui/layout.rs`, `src/ui/markdown.rs`, `src/utils/scroll.rs`).
- Modularized the chat loop with a keybinding registry, centralized redraw/height management, and event-driven rendering for smoother selection and paging (`src/ui/chat_loop`, `src/core/app.rs`).
- Expanded the streaming indicator into an eight-frame animation for a smoother pulse while responses generate (`src/ui/renderer.rs`).

### Fixed
- Normalized server-sent event parsing so streaming responses continue even when providers omit whitespace in `data:` lines (`src/ui/chat_loop/stream.rs`).
- Coalesced streamed chunks per render tick to avoid flickering partial updates and ensure markers finalize correctly (`src/ui/chat_loop/mod.rs`).
- Appended streamed content onto the in-progress assistant message instead of replacing it, preserving incremental updates during retries and long replies (`src/core/app/conversation.rs`).
- Deduplicated custom providers in the authentication removal menu so entries shared with built-ins only appear once (`src/auth/mod.rs`).
- Forced redraws and closures for stale OSC 8 hyperlinks, preventing scrolled links from lingering after the content changes (`src/ui/osc_backend.rs`).

### Performance
- Cached built-in provider definitions after the first load to avoid repeatedly parsing the embedded TOML (`src/core/builtin_providers.rs`).

## 0.3.5

### Added
- Add `/dump` command to export conversations to files (`src/commands/mod.rs`)

### Changed
- Include timestamps when starting, pausing, and resuming conversation logs (`src/utils/logging.rs`)

## 0.3.4

### Fixed

- Fixed cursor position calculation when text wraps in input fields (`src/core/text_wrapping.rs`)

## 0.3.3

### Fixed

- Fixed API token input handling with proper input saniitization

## 0.3.2

### Fixed
- Restored TLS support in reqwest client that was broken by dependency optimization

## 0.3.1

## Added
- Enhanced version information command (`--version`) with build details and git history in `src/cli/mod.rs`
- Reproducible build support using `SOURCE_DATE_EPOCH` and `VERGEN_IDEMPOTENT` environment variables

## Changed
- Optimized dependency features to reduce build time and binary size

## 0.3.0

### Added
- URL normalization utilities to prevent API endpoint issues (`src/utils/url.rs`)
- Support for additional built-in providers: Venice AI, Groq, Mistral, Cerebras (`src/builtins/models.toml`)
- Multi-line cursor navigation with Shift+Up/Down in input area (`src/ui/chat_loop.rs`)
- Token input masking with partial reveal toggle (F2) during authentication (`src/auth/mod.rs`)

### Changed
- Custom provider configurations now stored in main config file instead of keyring (`src/core/config.rs`)

### Fixed
- Prevent system messages (like `/help` output) from being sent to API (`src/core/app.rs`)

### Breaking
- Existing custom provider configurations will need to be recreated due to storage mechanism change

## 0.2.3

### Added
- Interactive provider selection command: `chabeau pick-default-provider` (`src/cli/pick_default_provider.rs`)

### Fixed
- Fixed streaming indicator display in combination with multi-line input

### Breaking
- Changed CLI command names for default configuration:
  - `set-default-model` → `pick-default-model`
  - Existing scripts using old command names will need to be updated

## 0.2.2

### Added
- Support for Ctrl+A/Ctrl+E, left/right to navigate input

### Fixed
- Sanitize pasted text input to prevent TUI corruption by converting tabs to spaces and filtering control characters (`src/ui/chat_loop.rs`)

## 0.2.1

### Added
- Multi-line input support with Alt+Enter keybinding (`src/core/app.rs`, `src/ui/chat_loop.rs`)
- Input area scrolling for long messages (`src/core/app.rs`)
- Dynamic input height expanding up to 6 lines (`src/core/app.rs`)

### Changed
- Improved terminal paste handling with bracketed paste mode support (`src/ui/chat_loop.rs`)
- Updated external editor shortcut from Ctrl+E to Ctrl+T (`README.md`, `src/cli/mod.rs`, `src/commands/mod.rs`)

## 0.2.0

### Added

- Added support for Anthropic API provider with proper header handling
- Configuration management system for default providers and models (`src/core/config.rs`)
- New CLI commands for setting default model & provider (`src/cli/set_default_model.rs`)

### Changed
- Default model selection now automatically uses newest available model when none is configured

## 0.1.0

Initial release with key features:
- Streaming OpenAI API support
- Scrolling dialog window
- Multi-provider support
- Store keys in system keyring
- External editor support
- Optionally log conversations
- Message retries
