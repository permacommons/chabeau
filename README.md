# Chabeau - Terminal Chat Interface

A full-screen terminal chat interface that uses the OpenAI streaming API for real-time conversations.

## Features

- Full-screen terminal UI with real-time streaming responses
- Configurable OpenAI model (defaults to gpt-4o)
- Environment variable configuration for API key and base URL
- Clean, responsive interface with color-coded messages
- Keyboard shortcuts for easy navigation

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

Run the application:
```bash
cargo run
```

Or with a specific model:
```bash
cargo run -- --model gpt-3.5-turbo
```

### Command Line Options

- `-m, --model <MODEL>`: Specify the OpenAI model to use (default: gpt-4o)

### Controls

- **Type**: Enter your message in the input field
- **Enter**: Send the message
- **Ctrl+C**: Quit the application
- **Backspace**: Delete characters in the input field

## Interface

The interface consists of two main areas:

1. **Chat Area**: Displays the conversation history with color-coded messages:
   - **Cyan/Bold**: Your messages
   - **Green**: Assistant responses

2. **Input Area**: Where you type your messages (highlighted in yellow when active)

## Example

```bash
# Set your API key
export OPENAI_API_KEY="sk-your-key-here"

# Run with default model (gpt-4o)
cargo run

# Run with a different model
cargo run -- --model gpt-3.5-turbo
```

## Dependencies

- `tokio`: Async runtime
- `reqwest`: HTTP client for API requests
- `ratatui`: Terminal UI framework
- `crossterm`: Cross-platform terminal manipulation
- `clap`: Command line argument parsing
- `serde`: JSON serialization/deserialization
- `futures-util`: Stream utilities

## License

This project is open source and available under the MIT License.
