# Chabeau Help

Thanks for using Chabeau, a Permacommons project.

Find a bug? Let us know: https://github.com/permacommons/chabeau/issues

## Keys

- Enter: Send message
- Alt+Enter or Ctrl+J: New line in input
- F4: Toggle compose mode (Enter=new line, Alt+Enter=send)
- Ctrl+R: Retry last response
- Ctrl+N: Re-run the most recent `/refine` prompt
- Ctrl+D: Exit when input is empty (prints transcript); otherwise [Del]
- Ctrl+C: Exit immediately (no transcript)
- Ctrl+P: Edit previous messages (select mode)
- Ctrl+X: Edit assistant messages (select mode)
- Ctrl+B: Select code blocks (copy `c`, save `s`)
- Ctrl+L: Clear status message
- Tab: Switch focus between transcript and input (`›` marks the active area, `·` the inactive) unless the current input starts with `/`, in which case it autocompletes slash commands. Tab stays on the transcript while you're in message-select (Ctrl+P/Ctrl+X) or block-select (Ctrl+B) mode until you exit or finish selecting.
- Ctrl+T: Open in external editor (requires `$EDITOR` to be set)
- Esc: Interrupt streaming / cancel modes
- Arrow keys: Move within the focused area; Up/Down scroll when the transcript is focused
- PageUp/PageDown: Scroll one page in history
- Home/End: Jump to top/bottom of history

## Picker Navigation

- Enter: Apply selection (session only)
- Alt+Enter or Ctrl+J: Apply selection and save to config
- Ctrl+O: Inspect full details (Esc=Back to picker)
- Esc: Cancel picker
- ↑/↓ or j/k: Navigate options
- Home/End: Jump to first/last option
- F6: Toggle sort mode
- Type: Filter options

## Tips

- Not all terminals support clickable hyperlinks. Even if yours does, you may need to hold a modifier key like [Ctrl] while clicking.
- Use `/log` to start logging from where you are.
- `/dump` creates a one-off snapshot of the _entire_ conversation so far.
- Use Ctrl+B to copy (`c`) or save (`s`) code blocks.
