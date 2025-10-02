# Chabeau Help

Thanks for using Chabeau! Find a bug? Let us know: https://github.com/permacommons/chabeau/issues

## Keys

- Enter: Send message
- Alt+Enter or Ctrl+J: New line in input
- F4: Toggle compose mode (Enter=new line, Alt+Enter=send)
- Ctrl+R: Retry last response
- Ctrl+D: Exit when input is empty; otherwise behaves like [Del]
- Ctrl+P: Edit previous messages (select mode)
- Ctrl+B: Select code blocks (copy `c`, save `s`)
- Ctrl+L: Clear status message
- Ctrl+T: Open in external editor (requires `$EDITOR` to be set)
- Esc: Interrupt streaming / cancel modes
- Up/Down/Mouse: Scroll history
- PageUp/PageDown: Scroll one page in history
- Home/End: Jump to top/bottom of history
- Shift+Cursor Keys: Move cursor in input

## Picker Navigation

- Enter: Apply selection (session only)
- Alt+Enter or Ctrl+J: Apply selection and save to config
- Esc: Cancel picker
- ↑/↓ or j/k: Navigate options
- Home/End: Jump to first/last option
- F6: Toggle sort mode
- Type: Filter options

## Commands

- `/theme` — Pick a theme (built-in or custom) with filtering and sorting
- `/theme <id>` — Apply a theme by id (persisted to config)
- `/model` — Pick a model from current provider with filtering, sorting, and metadata
- `/model <id>` — Switch to specified model (session only)
- `/provider` — Pick a provider with filtering and sorting
- `/provider <id>` — Switch to specified provider (session only)
  (Tip: On startup with multiple providers, Esc from model picker returns here.)
- `/markdown` — Toggle Markdown rendering. Persisted to config.
- `/syntax` — Toggle code syntax highlighting. Persisted to config.
- `/log <filename>` — Enable logging to file; `/log` toggles pause/resume
- `/dump [filename]` — Dump conversation to file (default: `chabeau-log-YYYY-MM-DD.txt`)

## Tips

- Use `/log` to start logging from where you are.
- `/dump` creates a one-off snapshot of the _entire_ conversation so far.
- Use Ctrl+B to copy (`c`) or save (`s`) code blocks.
- Chabeau will try to guess a good filename. If that already exists, it will prompt
  you to specify your own.

Chabeau is a Permacommons project. It is in the public domain, forever.
