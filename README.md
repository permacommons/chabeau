# Chabeau - Terminal Chat Interface

A full-screen terminal chat interface that uses the OpenAI streaming API for real-time conversations.

## Features

- Full-screen terminal UI with real-time streaming responses
- Configurable OpenAI model (defaults to gpt-4o)
- Environment variable configuration for API key and base URL
- Clean, responsive interface with color-coded messages
- Keyboard shortcuts for easy navigation
- **Chat logging functionality** - Log conversations to files with pause/resume capability
- Modular code architecture for maintainability

## Prerequisites

- Rust (latest stable version)
- OpenAI API key

## Installation

1. Clone this repository
2. Build the project:
   ```bash
   cargo build --release
   ```

## Configuration

Set the following environment variables:

- `OPENAI_API_KEY`: Your OpenAI API key (required)
- `OPENAI_BASE_URL`: Custom API base URL (optional, defaults to https://api.openai.com/v1)

Example:
```bash
export OPENAI_API_KEY="your-api-key-here"
export OPENAI_BASE_URL="https://api.openai.com/v1"  # Optional
```

## Usage

### Basic Usage

Run the application:
```bash
cargo run
```

Or with a specific model:
```bash
cargo run -- --model gpt-3.5-turbo
```

### Logging

Enable logging from the command line:
```bash
cargo run -- --log chat.log
```

Or enable/control logging during a session with commands:
- `/log <filename>` - Enable logging to specified file
- `/log` - Toggle logging pause/resume (if already enabled)

### Command Line Options

- `-m, --model <MODEL>`: Specify the OpenAI model to use (default: gpt-4o)
- `--log <FILE>`: Enable logging to specified file from startup

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
- `ui.rs` - Terminal user interface rendering
- `api.rs` - OpenAI API types and structures
- `logging.rs` - Chat logging functionality
- `commands.rs` - Chat command processing
- `message.rs` - Message data structures

This modular design makes the code easier to maintain, test, and extend.

## Example Sessions

### Basic chat:
```bash
export OPENAI_API_KEY="sk-your-key-here"
cargo run
```

### Chat with logging enabled:
```bash
export OPENAI_API_KEY="sk-your-key-here"
cargo run -- --log my-chat.log
```

### Chat with custom model and logging:
```bash
export OPENAI_API_KEY="sk-your-key-here"
cargo run -- --model gpt-3.5-turbo --log conversation.log
```

## Dependencies

- `tokio` - Async runtime
- `reqwest` - HTTP client for API requests
- `ratatui` - Terminal UI framework
- `crossterm` - Cross-platform terminal manipulation
- `clap` - Command line argument parsing
- `serde` - JSON serialization/deserialization
- `futures-util` - Stream utilities

## License

This project is open source and available under the MIT License.
