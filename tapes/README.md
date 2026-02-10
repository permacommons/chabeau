## VHS tapes

These are scripts for [VHS](https://github.com/charmbracelet/vhs), a CLI
video recording utility.

Each script showcases a Chabeau core feature. They should be regularly
re-run, both as an implicit integration test, and to reflect updates in the
UI.

The lib/ directory contains reusable scirpt blocks, nothing fancy.

For MCP-focused recordings, run `./mcp.sh mcp.tape` (or another tape path)
from this directory. Use `./mcp.sh --no-prompt mcp.tape` to skip the final
interactive cleanup pause. The wrapper temporarily configures the
[research-friend MCP server](https://github.com/permacommons/mcp-research-friend) for testing,
preloads configured PDFs into `~/.research-friend/inbox`, and restores your
original MCP settings afterward.

To avoid bloating this GitHub repository, actual recordings are uploaded to
https://permacommons.org/
