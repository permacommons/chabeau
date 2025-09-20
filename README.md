# Chabeau - Terminal Chat Interface

![Chabeau in action, courtesy of VHS](vhs.gif)

**NOTE:** This is pre-alpha software. It has only been tested on Linux.

A full-screen terminal chat interface that connects to various AI APIs for real-time conversations with secure credential management.

Chabeau is not a coding agent, nor does it aspire to be one. Instead, it brings the conveniences of web-based chat UIs to the terminal, with your favorite editor for long prompts.

![Our friendly mascot](chabeau-mascot-small.png)

## Features

- Full-screen terminal UI with real-time streaming responses
- Efficient byte-level parsing for smooth streaming output
- Secure API key storage in system keyring with config-based provider management
- Multi-line input (IME-friendly)
- Multiple OpenAI-compatible providers (OpenAI, OpenRouter, Poe, Anthropic, Venice AI, Groq, Mistral, Cerebras, custom)
- Interactive theme and model pickers with filtering and sorting
- Message retry and external editor support
- Conversation logging with pause/resume
- Markdown rendering in the chat area (headings, lists, quotes, tables, inline/fenced code)
- Syntax highlighting for fenced code blocks (Python, Bash, JavaScript, and more)
- Inline block selection (Ctrl+B) to copy or save fenced code blocks

## Quick Start

### Installation
```bash
cargo install chabeau
```

### Setup Authentication
```bash
chabeau auth    # Interactive setup for OpenAI, OpenRouter, Poe, Anthropic, Venice AI, Groq, Mistral, Cerebras, or custom providers
```

### Start Chatting
```bash
chabeau         # Uses defaults; opens pickers when needed
```

Inside the TUI, use `/provider` and `/model` to switch.

## How does auto-selection work?

We try to do what makes the most sense:

- Provider: If you've picked a default provider, Chabeau uses it. If not, but exactly one provider has auth (keyring or environment), Chabeau uses that one. If multiple providers are available and none is default, Chabeau launches the TUI and opens the provider picker. If no providers are configured, it will prompt you to configure auth and exit.
- Model: If the chosen provider has no default model configured, Chabeau launches into the model picker. The newest model (when metadata is available) is highlighted. Cancelling this picker at startup exits the app; if multiple providers are available, cancelling returns to the provider picker instead.

## Usage

### Basic Commands
```bash
chabeau                              # Start chat with defaults (pickers on demand)
chabeau --provider openai            # Use specific provider
chabeau --model gpt-3.5-turbo        # Use specific model
chabeau --log conversation.log       # Enable logging
```

### Discovery
```bash
chabeau -p                           # List providers and auth status
chabeau -m                           # List available models
chabeau -p openrouter -m             # List models for specific provider
```

### Authentication Management
```bash
chabeau auth                         # Set up authentication
chabeau deauth                       # Remove authentication (interactive)
chabeau deauth --provider openai     # Remove specific provider
```

### Environment Variables (--env or fallback)
Environment variables are used only if no providers are configured, or when you pass `--env`.

```bash
export OPENAI_API_KEY="your-api-key-here"
export OPENAI_BASE_URL="https://api.openai.com/v1"  # Optional
chabeau --env     # Force using env vars even if providers are configured
```

Environment variable values can make their way into shell histories or other places they shouldn't, 
so using the keyring is generally advisable.

### Configuration

Chabeau supports configuring default providers and models for a smoother experience. 
The easiest way to do so is via the `/model` and `/provider` commands in the TUI,
which open interactive pickers. Use Alt+Enter to persist a choice to the config.

You can also do it on the command line:

```bash
chabeau set default-provider openai     # Set default provider
chabeau set default-model openai gpt-4o # Set default model for a provider
```

There are even simplified interactive selectors:

```bash
chabeau pick-default-provider            # Interactive provider selection
chabeau pick-default-model               # Interactive model selection
chabeau pick-default-model --provider openai  # Select model for specific provider
```

View current configuration:
```bash
chabeau set default-provider            # Show current configuration
```

### Themes

Chabeau includes built-in themes to customize the TUI appearance.


- Use `/theme` in the TUI to pick a theme, and use Alt+Enter to persist it to the config.
- You can also use the commmand line to set a default theme, e.g.:
  - Set a theme: `chabeau set theme dark`
  - List themes: `chabeau themes` (shows built-in and custom, marks current)
- Unset theme (revert to default): `chabeau unset theme`

Auto-detection: when no theme is set in your config, Chabeau tries to infer a sensible default from the OS preference (e.g., macOS, Windows, GNOME). If no hint is available, it defaults to the dark theme.

Custom themes:
- You can define custom themes in your config file (`~/.config/chabeau/config.toml`) under `[[custom_themes]]` entries with fields matching the built-ins (see [src/builtin_themes.toml](src/builtin_themes.toml) for examples).
- Once added, set them with `chabeau set theme <your-theme-id>`.

### Preferences

You can persist UI preferences in your config file (`~/.config/chabeau/config.toml`).

- `markdown = true|false` — Enable/disable Markdown rendering. Default: `true`.
- `syntax = true|false` — Enable/disable syntax highlighting for fenced code blocks. Default: `true`.

At runtime, use chat commands to toggle and persist:
- `/markdown on|off|toggle`
- `/syntax on|off|toggle`

Syntax colors adapt to the active theme (dark/light) and use the theme’s code block background for consistent contrast.

### Color Support

Chabeau detects terminal color depth and adapts themes accordingly:

- Truecolor: if `COLORTERM` contains `truecolor`/`24bit`, Chabeau uses 24‑bit RGB.
- 256 colors: if `TERM` contains `256color`, RGB colors are quantized to the xterm‑256 palette.
- ANSI 16: otherwise, colors are mapped to the nearest 16 ANSI colors.

You can force a mode with `CHABEAU_COLOR=truecolor|256|16` if needed.

## Interface Controls

See [the built-in help](src/ui/builtin_help.md) for a full list of keyboard controls and commands.

Most should be intuitive. A couple of choices may be a bit jarring at first:

- Alt+Enter to start a new line: We've found this to be most reliable across terminals.
- Shift+Cursor to move around in input: This is so the cursor keys can be used at any time to scroll in the output area.

Feedback and suggestions are always welcome!

### External Editor
Set `EDITOR` environment variable:
```bash
export EDITOR=nano          # or vim, code, etc.
export EDITOR="code --wait" # VS Code with wait
```

## Architecture

Modular design with focused components:

- `main.rs` - Entry point
- `builtin_models.toml` - Build-time configuration for supported providers
- `cli/` - Command-line interface parsing and handling
  - `mod.rs` - CLI argument parsing and command dispatching
  - `model_list.rs` - Model listing functionality
  - `provider_list.rs` - Provider listing functionality
  - `pick_default_model.rs` - Default model configuration
  - `pick_default_provider.rs` - Default provider configuration
- `core/` - Core application components
  - `mod.rs` - Core module declarations
  - `app.rs` - Core application state
  - `builtin_providers.rs` - Built-in provider configuration (loads from `builtin_models.toml`)
  - `config.rs` - Configuration management
  - `message.rs` - Message data structures
- `auth/` - Authentication and provider management
  - `mod.rs` - Authentication manager implementation
- `api/` - API types and models
  - `mod.rs` - API data structures
  - `models.rs` - Model fetching and sorting functionality
- `ui/` - Terminal interface rendering
  - `mod.rs` - UI module declarations
  - `chat_loop.rs` - Main chat event loop and UI rendering
  - `renderer.rs` - Terminal interface rendering
  - `markdown.rs` - Lightweight Markdown rendering tuned for terminals
- `utils/` - Utility functions and helpers
  - `mod.rs` - Utility module declarations
  - `editor.rs` - External editor integration
  - `logging.rs` - Chat logging functionality
  - `scroll.rs` - Text wrapping and scroll calculations
  - `clipboard.rs` - Cross-platform clipboard helper
- `commands/` - Chat command processing
  - `mod.rs` - Command processing implementation

### Built-in Provider Configuration

Chabeau uses a build-time configuration system for built-in providers. The `builtin_models.toml` file defines supported providers with their IDs, display names, base URLs, and authentication modes.

This configuration is embedded into the binary at compile time, eliminating runtime file dependencies while allowing easy modification of supported providers during development.

## Development

### Running Tests
```bash
cargo test                    # All tests
cargo test scroll::           # Scroll functionality tests
cargo test --release          # Faster execution
```

### Performance

Chabeau includes lightweight performance checks in the unit test suite and supports optional Criterion benches.

- Built-in perf checks (unit tests):
  - Short history prewrap (50 iters, ~60 lines): warns at ≥ 90ms; fails at ≥ 200ms.
  - Large history prewrap (20 iters, ~400 lines): warns at ≥ 400ms; fails at ≥ 1000ms.
  - Run with: `cargo test` (warnings print to stderr; tests only fail past the fail thresholds).

- Optional benches (release mode) using Criterion 0.7:
  - A `render_cache` bench is checked in to validate the cached prewrapped rendering path.
  - Run: `cargo bench`
  - Reports: `target/criterion/` (HTML under `report/index.html`).
  - To add new benches, create files under `benches/` (e.g., `benches/my_bench.rs`) and use Criterion’s `criterion_group!/criterion_main!`.
  - Benches import internal modules via `src/lib.rs` (e.g., `use chabeau::...`).

### Key Dependencies
- `tokio` - Async runtime
- `ratatui` - Terminal UI framework
- `reqwest` - HTTP client
- `keyring` - Secure credential storage
- `clap` - Command line parsing

## License

CC0 1.0 Universal (Public Domain)
### Edit Previous Messages (Ctrl+P)

- Press `Ctrl+P` to enter edit-select mode. The most recent user message is highlighted. The input area locks and shows instructions.
- Navigate between your messages with `Up/Down` or `j/k`.
- You can also press `Ctrl+P` repeatedly to cycle upward through your messages (wraps at the top).
- Press `Enter` to delete the selected user message and all messages below it, and put its content into the input area for editing and resending.
- Press `e` to edit the selected message in place: the input area is populated so you can edit, then press `Enter` to apply changes back to history (no send, no deletion). Use `Ctrl+R` afterwards to retry from that point if desired.
- Press `Delete` to delete the selected user message and everything below it without populating the input.
- Press `Esc` to cancel and return to normal typing.
