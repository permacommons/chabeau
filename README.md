# Chabeau - Terminal Chat Interface

![Chabeau in action](vhs.gif)

**NOTE:** This is pre-alpha software. It has only been tested on Linux.

A full-screen terminal chat interface that connects to various AI APIs for real-time conversations with secure credential management.

Chabeau is not a coding agent, nor does it aspire to be one. Instead, it brings the conveniences of web-based chat UIs to the terminal, with your favorite editor for long prompts.

![Our friendly mascot](chabeau-mascot-small.png)

## Features

- Full-screen terminal UI with real-time streaming responses
- Multiple OpenAI-compatible providers (OpenAI, OpenRouter, Poe, Anthropic, custom)
- Secure API key storage in system keyring
- Message retry and external editor support
- Conversation logging with pause/resume

## Quick Start

### Installation
```bash
cargo install chabeau
```

### Setup Authentication
```bash
chabeau auth    # Interactive setup for OpenAI, OpenRouter, Poe, Anthropic, or custom providers
```

### Start Chatting
```bash
chabeau         # Uses default provider and model (automatically selects newest model if no default configured)
```

**Note:** Chabeau will use your configured default provider and model if set. If no default model is configured for your provider, Chabeau will automatically fetch and use the newest available model from that provider. If you haven't configured a default provider, Chabeau will use the first available authenticated provider.

## Usage

### Basic Commands
```bash
chabeau                              # Start chat with defaults
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

### Environment Variables (Fallback)
If no authentication is configured:
```bash
export OPENAI_API_KEY="your-api-key-here"
export OPENAI_BASE_URL="https://api.openai.com/v1"  # Optional
```

### Configuration

Chabeau supports configuring default providers and models for a smoother experience:

```bash
chabeau set default-provider openai     # Set default provider
chabeau set default-model openai gpt-4o # Set default model for a provider
```

You can also use the interactive selectors:
```bash
chabeau pick-default-provider            # Interactive provider selection
chabeau pick-default-model               # Interactive model selection
chabeau pick-default-model --provider openai  # Select model for specific provider
```

View current configuration:
```bash
chabeau set default-provider            # Show current configuration
```

**Note:** The `set default-model` command now accepts provider and model names without requiring quotes, making it easier to use.

## Interface Controls

| Key | Action |
|-----|--------|
| **Type** | Enter message |
| **Enter** | Send message |
| **Alt+Enter** | Insert newline in input |
| **Ctrl+A** | Move cursor to beginning of input |
| **Ctrl+E** | Move cursor to end of input |
| **Left/Right** | Move cursor left/right in input |
| **Up/Down/Mouse** | Scroll chat history |
| **Ctrl+C** | Quit |
| **Ctrl+R** | Retry last response |
| **Ctrl+T** | Open external editor |
| **Esc** | Interrupt streaming |

### Chat Commands
- `/help` - Show extended help
- `/log <filename>` - Enable/switch log file
- `/log` - Toggle logging pause/resume

### External Editor
Set `EDITOR` environment variable:
```bash
export EDITOR=nano          # or vim, code, etc.
export EDITOR="code --wait" # VS Code with wait
```

## Interface Layout

- **Chat Area**: Color-coded conversation history
  - **Cyan/Bold**: Your messages
  - **White**: Assistant responses
  - **Gray**: System messages
- **Input Area**: Message composition (yellow highlight)
- **Title Bar**: Version, provider, model, logging status

## Architecture

Modular design with focused components:

- `main.rs` - Entry point
- `cli/` - Command-line interface parsing and handling
  - `mod.rs` - CLI argument parsing and command dispatching
  - `model_list.rs` - Model listing functionality
  - `provider_list.rs` - Provider listing functionality
  - `pick_default_model.rs` - Default model configuration
  - `pick_default_provider.rs` - Default provider configuration
- `core/` - Core application components
  - `mod.rs` - Core module declarations
  - `app.rs` - Core application state
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
- `utils/` - Utility functions and helpers
  - `mod.rs` - Utility module declarations
  - `editor.rs` - External editor integration
  - `logging.rs` - Chat logging functionality
  - `scroll.rs` - Text wrapping and scroll calculations
- `commands/` - Chat command processing
  - `mod.rs` - Command processing implementation

## Development

### Running Tests
```bash
cargo test                    # All tests
cargo test scroll::           # Scroll functionality tests
cargo test --release          # Faster execution
```

### Key Dependencies
- `tokio` - Async runtime
- `ratatui` - Terminal UI framework
- `reqwest` - HTTP client
- `keyring` - Secure credential storage
- `clap` - Command line parsing

## License

CC0 1.0 Universal (Public Domain)
