Output recordings/refine.mp4
Source lib/preamble.tape

Type "In one sentence, what is the meaning of life?"
Source lib/wait-turn.tape

Type "/refine more focused on food, more emoji"
Source lib/wait-turn.tape

Type "By pressing Ctrl+N, we can recursively refine the same response."
Source lib/del-input.tape

Ctrl+N
Wait+Screen /Ctrl\+C\=quit/
Sleep 1

Ctrl+N
Wait+Screen /Ctrl\+C\=quit/
Sleep 3

Type "Tell me a joke."
Source lib/wait-turn.tape

Type "/refine truly original, never before told"
Source lib/wait-turn.tape

Type "By pressing Ctrl+R, we retry the refinement against the original (no recursion)."
Source lib/del-input.tape

Ctrl+R
Wait+Screen /Ctrl\+C\=quit/
Sleep 1

Ctrl+R
Wait+Screen /Ctrl\+C\=quit/
Sleep 5
