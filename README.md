# Chabeau - Terminal Chat Interface

![Chabeau rendering a complex table](chabeau.png)

**See several of Chabeau's features in action in [this short video](https://permacommons.org/videos/chabeau-0.4.0.mp4).**

A full-screen terminal chat interface that connects to various AI APIs for real-time conversations.

Chabeau is not a coding agent, nor does it aspire to be one. Instead, it brings the conveniences of web-based chat UIs to the terminal. Its focus is on conversation and speed.

**NOTE:** This is pre-alpha software. It has been tested on Linux and macOS.

![Our friendly mascot](chabeau-mascot-small.png)

## Features

- Full-screen terminal UI with real-time streaming responses
- Markdown rendering in the chat area (headings, lists, quotes, tables, callouts, superscript/subscript, inline/fenced code) with clickable OSC 8 hyperlinks
- Built-in support for many common providers (OpenAI, OpenRouter, Poe, Anthropic, Venice AI, Groq, Mistral, Cerebras)
- Support for quick custom configuration of new OpenAI-compatible providers
- Interactive dialogs for selecting models (e.g., Claude vs. GPT-5) and providers
- Extensible theming system that degrades gracefully to terminals with limited color support
- Secure API key storage in system keyring with config-based provider management
- Multi-line input (IME-friendly) with compose mode for longer responses
- Message retry and message editing
- Conversation logging with pause/resume; quick `/dump` of contents to a file
- Syntax highlighting for fenced code blocks (Python, Bash, JavaScript, and more)
- Inline block selection (Ctrl+B) to copy or save fenced code blocks

For features under consideration, see [WISHLIST.md](WISHLIST.md)

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

Inside the TUI, use `/provider` and `/model` to switch, and `/help` to see a full breakdown of commands and keyboard shortcuts.

## Usage

### Basic Commands
```bash
chabeau                              # Start chat with defaults (pickers on demand)
chabeau --provider openai            # Use specific provider
chabeau --model gpt-5                # Use specific model
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
which open interactive pickers. Use Alt+Enter or Ctrl+J to persist a choice to the config.

You can also do it on the command line:

```bash
chabeau set default-provider openai     # Set default provider
chabeau set default-model openai gpt-4o # Set default model for a provider
```

There are also simplified interactive selectors:

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


- Use `/theme` in the TUI to pick a theme, and use Alt+Enter or Ctrl+J to persist it to the config.
- You can also use the commmand line to set a default theme, e.g.:
  - Set a theme: `chabeau set theme dark`
  - List themes: `chabeau themes` (shows built-in and custom, marks current)
- Unset theme (revert to default): `chabeau unset theme`

Auto-detection: when no theme is set in your config, Chabeau tries to infer a sensible default from the OS preference (e.g., macOS, Windows, GNOME). If no hint is available, it defaults to the dark theme.

Custom themes:
- You can define custom themes in your config file (`~/.config/chabeau/config.toml`) under `[[custom_themes]]` entries with fields matching the built-ins (see [src/builtins/themes.toml](src/builtins/themes.toml) for examples).
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

See [the built-in help](src/builtins/help.md) for a full list of keyboard controls and commands.

Most should be intuitive. A couple of choices may be a bit jarring at first:

- Alt+Enter (or Ctrl+J) to start a new line: We've found this to be most reliable across terminals.
- Compose mode (F4) flips the defaults: Enter inserts a newline, Alt+Enter/Ctrl+J sends, arrow keys stay in the input, and Shift+arrow scrolls the transcript.

Feedback and suggestions are always welcome!

## Mousewheel Use

We avoid capturing the mouse so that selection operation (copy/paste) work without issues. Some terminals treat mousewheel events as cursor key input,
so scrolling the mousewheel will scroll the conversation.

In other terminals, scrolling the mousewheel may reveal the contents of your terminal prior to your launch of Chabeau. In that case, we recommend using the cursor keys or PgUp/PgDn instead.

### External Editor
Set `EDITOR` environment variable:
```bash
export EDITOR=nano          # or vim, code, etc.
export EDITOR="code --wait" # VS Code with wait
```

Once the variable is set, you can compose messages using the external editor via Ctrl+T.

## Architecture

Modular design with focused components:

- `main.rs` - Entry point
- `builtins/` - Build-time assets embedded into the binary
  - `models.toml` - Supported provider definitions
  - `themes.toml` - Built-in UI themes
  - `help.md` - In-app keyboard shortcut and command reference
- `cli/` - Command-line interface parsing and handling
  - `mod.rs` - CLI argument parsing and command dispatching
  - `model_list.rs` - Model listing functionality
  - `provider_list.rs` - Provider listing functionality
  - `pick_default_model.rs` - Default model configuration
  - `pick_default_provider.rs` - Default provider configuration
- `core/` - Core application components
  - `app/` - Application state and controllers
    - `mod.rs` - App struct and module exports
    - `conversation.rs` - Conversation controller for chat flow, retries, and streaming helpers
    - `session.rs` - Session bootstrap and provider/model state
    - `settings.rs` - Theme and provider controllers
    - `ui_state.rs` - UI state management and text input helpers
  - `chat_stream.rs` - Shared streaming service that feeds responses to the app, UI, and loggers
  - `builtin_providers.rs` - Built-in provider configuration (loads from `builtins/models.toml`)
  - `config.rs` - Configuration management
  - `message.rs` - Message data structures
- `auth/` - Authentication and provider management
  - `mod.rs` - Authentication manager implementation
- `api/` - API types and models
  - `mod.rs` - API data structures
  - `models.rs` - Model fetching and sorting functionality
- `ui/` - Terminal interface rendering
  - `mod.rs` - UI module declarations
  - `chat_loop/` - Mode-aware chat loop orchestrating UI flows, keybindings, and command routing
  - `layout.rs` - Shared width-aware layout engine for Markdown and plain text
  - `markdown.rs` / `markdown_wrap.rs` - Markdown renderer and wrapping helpers that emit span metadata
  - `renderer.rs` - Terminal interface rendering (chat area, input, pickers)
  - `osc_backend.rs` / `osc_state.rs` / `osc.rs` - Crossterm backend wrapper that emits OSC 8 hyperlinks
  - `picker.rs` / `appearance.rs` / `theme.rs` - Picker controls and theming utilities
- `utils/` - Utility functions and helpers
  - `mod.rs` - Utility module declarations
  - `color.rs` - Terminal color detection and palette quantization
  - `editor.rs` - External editor integration
  - `logging.rs` - Chat logging functionality
  - `scroll.rs` - Text wrapping and scroll calculations
  - `clipboard.rs` - Cross-platform clipboard helper
- `commands/` - Chat command processing and registry-driven dispatch
  - `mod.rs` - Command handlers and dispatcher
  - `registry.rs` - Static command metadata registry

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

## License

CC0 1.0 Universal (Public Domain)