# Changelog

## 0.3.1

## Added
- Enhanced version information command (`--version`) with build details and git history in `src/cli/mod.rs`
- Reproducible build support using `SOURCE_DATE_EPOCH` and `VERGEN_IDEMPOTENT` environment variables

## Changed
- Optimized dependency features to reduce build time and binary size

## 0.3.0

### Added
- URL normalization utilities to prevent API endpoint issues (`src/utils/url.rs`)
- Support for additional built-in providers: Venice AI, Groq, Mistral, Cerebras (`builtin_models.toml`)
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
