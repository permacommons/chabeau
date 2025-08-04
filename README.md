# Chabeau - Terminal Chat Interface

**NOTE:** This is pre-alpha software. It has only been tested on Linux.

A full-screen terminal chat interface that connects to various AI APIs for real-time conversations with secure credential management.

## Features

- Full-screen terminal UI with real-time streaming responses
- Configure as many OpenAI-compatible providers as you like (e.g., OpenAI API, Poe, OpenRouter)
- API keys are securely stored in the system keyring
- Message retry
- External editor support
- Log conversations to files with pause/resume capability

## Prerequisites

- Rust (latest stable version)
- API key for at least one supported provider

## Installation

Install directly from crates.io:
```bash
cargo install chabeau
```

Or clone and install locally:
```bash
git clone https://github.com/your-username/chabeau
cd chabeau
cargo install --path .
```

## Authentication Setup

**Recommended**: Use the built-in authentication system to securely store your API credentials:

```bash
chabeau auth
```

This will guide you through setting up authentication for:
1. **OpenAI** (https://api.openai.com/v1)
2. **OpenRouter** (https://openrouter.ai/api/v1)
3. **Poe** (https://api.poe.com/v1)
4. **Custom providers** (specify your own name and base URL)

Your credentials are stored securely in your system's keyring and can be used across sessions.

### Environment Variables (Fallback)

If no authentication is configured, Chabeau will fall back to environment variables:

- `OPENAI_API_KEY`: Your OpenAI API key
- `OPENAI_BASE_URL`: Custom API base URL (optional, defaults to https://api.openai.com/v1)

Example:
```bash
export OPENAI_API_KEY="your-api-key-here"
export OPENAI_BASE_URL="https://api.openai.com/v1"  # Optional
```

## Usage

### Authentication Management

**Set up authentication** (recommended first step):
```bash
chabeau auth
```

**Remove authentication** for providers:
```bash
chabeau deauth                     # Interactive menu to select provider
chabeau deauth --provider openai   # Remove specific provider
chabeau deauth --provider mycustom # Remove custom provider (completely)
```

**Check provider status**:
```bash
chabeau -p                         # List all providers and their auth status
```

Note: When removing custom providers with `deauth`, both the authentication token and the provider definition are completely removed from the system keyring.

### Basic Usage

Run the chat interface (default command):
```bash
chabeau
```

Or explicitly use the chat command:
```bash
chabeau chat
```

### Provider Selection

Use a specific provider:
```bash
chabeau --provider openai
chabeau --provider openrouter
chabeau --provider poe
chabeau --provider mycustom  # If you set up a custom provider
```

If no provider is specified, Chabeau will automatically use the first available authentication in this order:
1. OpenAI
2. OpenRouter
3. Poe
4. Custom providers
5. Environment variables (fallback)

### Model and Logging Options

Specify a model:
```bash
chabeau --model gpt-4o
```

Enable logging from startup:
```bash
chabeau --log chat.log
```

Combine options:
```bash
chabeau --provider openrouter --model gpt-4o --log conversation.log
```

### Runtime Logging Control

Enable/control logging during a session with commands:
- `/log <filename>` - Enable logging to specified file
- `/log` - Toggle logging pause/resume (if already enabled)

### Model and Provider Discovery

**List available models** for the default provider:
```bash
chabeau -m                        # List models (newest first)
```

**List available models** for a specific provider:
```bash
chabeau -p openai -m              # List OpenAI models
chabeau -p openrouter -m          # List OpenRouter models
```

**List available providers** and their authentication status:
```bash
chabeau -p                        # List all providers
```

### Command Line Options

**Global options (work with or without 'chat' command):**
- `-m, --model [MODEL]`: Specify the model to use, or list available models if no model specified (default: gpt-4o)
- `-p, --provider [PROVIDER]`: Specify provider, or list available providers if no provider specified
- `-l, --log <FILE>`: Enable logging to specified file from startup

**Commands:**
- `auth` - Interactive authentication setup
- `deauth` - Remove authentication for providers
- `chat` - Start the chat interface (default command)

**Examples:**
```bash
# These are equivalent (chat is the default command):
chabeau --model gpt-3.5-turbo --provider openrouter
chabeau chat --model gpt-3.5-turbo --provider openrouter

# Short options work too:
chabeau -m gpt-4 -p openai -l chat.log

# Case-insensitive provider names:
chabeau -p OpenAI    # Same as -p openai
chabeau -p OPENROUTER # Same as -p openrouter
```

### Controls

- **Type**: Enter your message in the input field
- **Enter**: Send the message
- **Up/Down/Mouse**: Scroll through chat history
- **Ctrl+C**: Quit the application
- **Ctrl+E**: Open external editor (requires EDITOR environment variable)
- **Backspace**: Delete characters in the input field
- **Esc**: Interrupt streaming response
- **Ctrl+R**: Retry the last bot response (regenerates with same context)

### External Editor Support

Chabeau supports opening an external text editor for composing longer messages:

- **Ctrl+E**: Opens your configured editor with the current input content
- Requires the `EDITOR` environment variable to be set
- When you save and exit the editor, the content is sent immediately
- Empty files or files with only whitespace are ignored

**Setup:**
```bash
export EDITOR=nano          # Use nano
export EDITOR=vim           # Use vim
export EDITOR=code          # Use VS Code
export EDITOR="code --wait" # Use VS Code and wait for window to close
```

### Chat Commands

- `/help` - Show extended help with all keyboard shortcuts and commands
- `/log <filename>` - Enable logging to specified file
- `/log` - Toggle logging pause/resume

## Interface

The interface consists of two main areas:

1. **Chat Area**: Displays the conversation history with color-coded messages:
   - **Cyan/Bold**: Your messages (prefixed with "You:")
   - **White**: Assistant responses
   - **Gray**: System messages (logging status, etc.)

2. **Input Area**: Where you type your messages (highlighted in yellow when active)

The title bar shows the version number, current provider, model, and logging status.

## Logging Features

- **Command-line activation**: Start with logging enabled using `--log filename`
- **Runtime control**: Use `/log filename` to enable or switch log files during a session
- **Pause/Resume**: Use `/log` to pause and resume logging without losing the file
- **Multiple files**: Switch between different log files during a session
- **Exact formatting**: Logs preserve the exact formatting as displayed on screen
- **Automatic spacing**: Maintains proper spacing between messages in the log

## Code Architecture

The codebase is organized into focused modules:

- `main.rs` - Application entry point and main event loop
- `app.rs` - Core application state and logic
- `auth.rs` - Authentication and provider management using system keyring
- `ui.rs` - Terminal user interface rendering
- `api.rs` - API types and structures for various providers
- `logging.rs` - Chat logging functionality
- `commands.rs` - Chat command processing
- `message.rs` - Message data structures
- `scroll.rs` - Scroll calculations and line wrapping logic

This modular design makes the code easier to maintain, test, and extend.

## Testing

Chabeau includes comprehensive unit tests, particularly for the scroll functionality which handles complex text wrapping and positioning calculations.

### Running Tests

**Run all tests:**
```bash
cargo test
```

**Run tests with verbose output:**
```bash
cargo test -- --nocapture
```

**Run only scroll-related tests:**
```bash
cargo test scroll::
```

**Run tests in release mode (faster execution):**
```bash
cargo test --release
```

**Run tests quietly (minimal output):**
```bash
cargo test --quiet
```


### Test Organization

The test suite covers:

- **Scroll calculations**: Word-based text wrapping that matches ratatui's behavior
- **Line building**: Message formatting and display line generation
- **Scroll positioning**: Automatic scroll-to-bottom and scroll-to-message functionality
- **Edge cases**: Empty messages, long paragraphs, zero-width terminals, whitespace handling

Key test areas:
- `scroll::tests::*` - Comprehensive scroll functionality tests
- Text wrapping with various terminal widths
- Message formatting for different roles (user, assistant, system)
- Scroll offset calculations for navigation and auto-scroll

### Debugging Tests

**Run a specific test with debug output:**
```bash
cargo test test_word_wrapping_with_long_paragraph -- --nocapture
```

**Run tests with backtraces on failure:**
```bash
RUST_BACKTRACE=1 cargo test
```

**Run tests with full backtraces:**
```bash
RUST_BACKTRACE=full cargo test
```

## Example Sessions

### First-time setup:
```bash
# Install Chabeau
cargo install --path .

# Set up authentication
chabeau auth

# Check what providers are available
chabeau -p

# See what models are available
chabeau -m

# Start chatting
chabeau
```

### Discovery and exploration:
```bash
# List all providers and their status
chabeau -p

# List models for the default provider
chabeau -m

# List models for a specific provider
chabeau -p openrouter -m

# List models for OpenAI specifically
chabeau -p openai -m
```

### Using different providers:
```bash
# Use OpenRouter with a specific model
chabeau chat --provider openrouter --model claude-3-sonnet

# Use a custom provider you set up
chabeau chat --provider myapi --model custom-model
```

### With logging:
```bash
# Basic chat with logging
chabeau chat --log my-chat.log

# Specific provider with logging
chabeau chat --provider openai --model gpt-4 --log conversation.log
```

### Fallback to environment variables:
```bash
# If no auth is configured, use environment variables
export OPENAI_API_KEY="sk-your-key-here"
chabeau
```

## Dependencies

- `tokio` - Async runtime
- `reqwest` - HTTP client for API requests
- `ratatui` - Terminal UI framework
- `crossterm` - Cross-platform terminal manipulation
- `clap` - Command line argument parsing
- `serde` - JSON serialization/deserialization
- `futures-util` - Stream utilities
- `keyring` - Secure credential storage in system keyring

## License

This project is open source and available under the MIT License.
