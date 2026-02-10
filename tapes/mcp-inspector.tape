Output recordings/mcp-inspector.mp4
Source lib/preamble.tape

Wait+Screen@30s /connected/
Type "Top 5 current events reported by Wikipedia; include links to WP articles"
Enter
Source lib/wait-turn.tape
Sleep 10
Type "We can press Ctrl+O to view the tool response, decode it, or look at the original request."
Sleep 3
Source lib/del-input.tape
Ctrl+O
Sleep 5
Type "D"
Down@200 ms 10
Sleep 5
Tab
Sleep 5
Ctrl+C