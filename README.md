# Chabeau - Terminal Chat Interface

A full-screen terminal chat interface that connects to various AI APIs for real-time conversations with secure credential management.

## Features

- Full-screen terminal UI with real-time streaming responses
- **Secure authentication** using system keyring for API credentials
- **Multiple provider support**: OpenAI, OpenRouter, Poe, and custom providers
- Configurable models (defaults to gpt-4o)
- Provider selection with automatic fallback
- Clean, responsive interface with color-coded messages
- Keyboard shortcuts for easy navigation
- **Chat logging functionality** - Log conversations to files with pause/resume capability
- Modular code architecture for maintainability

## Prerequisites

- Rust (latest stable version)
- API key for at least one supported provider

## Installation

Install directly from the repository:
```bash
cargo install --git https://github.com/your-username/chabeau
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

### Authentication Command

Set up secure authentication (recommended first step):
```bash
chabeau auth
```

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
chabeau chat --provider openai
chabeau chat --provider openrouter
chabeau chat --provider poe
chabeau chat --provider mycustom  # If you set up a custom provider
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
chabeau chat --model gpt-3.5-turbo
```

Enable logging from startup:
```bash
chabeau chat --log chat.log
```

Combine options:
```bash
chabeau chat --provider openrouter --model gpt-4 --log conversation.log
```

### Runtime Logging Control

Enable/control logging during a session with commands:
- `/log <filename>` - Enable logging to specified file
- `/log` - Toggle logging pause/resume (if already enabled)

### Command Line Options

**Global options (work with or without 'chat' command):**
- `-m, --model <MODEL>`: Specify the model to use (default: gpt-4o)
- `-p, --provider <PROVIDER>`: Specify provider (openai, openrouter, poe, or custom name)
- `-l, --log <FILE>`: Enable logging to specified file from startup

**Commands:**
- `auth` - Interactive authentication setup
- `chat` - Start the chat interface (default command)
- `providers` - List available providers and their authentication status

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
- **Backspace**: Delete characters in the input field
- **Esc**: Interrupt streaming response
- **Ctrl+R**: Retry the last bot response (regenerates with same context)

### Chat Commands

- `/log <filename>` - Enable logging to specified file
- `/log` - Toggle logging pause/resume

## Interface

The interface consists of two main areas:

1. **Chat Area**: Displays the conversation history with color-coded messages:
   - **Cyan/Bold**: Your messages (prefixed with "You:")
   - **White**: Assistant responses
   - **Gray**: System messages (logging status, etc.)

2. **Input Area**: Where you type your messages (highlighted in yellow when active)

The title bar shows the current model and logging status.

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

This modular design makes the code easier to maintain, test, and extend.

## Example Sessions

### First-time setup:
```bash
# Install Chabeau
cargo install --path .

# Set up authentication
chabeau auth

# Start chatting
chabeau
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
