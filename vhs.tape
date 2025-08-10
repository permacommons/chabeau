# Created with https://github.com/charmbracelet/vhs
# Run in a privileged Docker container based on Debian Trixie like so:
#
# docker run -it --cap-add=SYS_ADMIN --hostname core -v .:/vhs --security-opt seccomp=unconfined vhs /bin/bash
# 
# To get VHS running, had to install Homewbrew, git, build-essentials, and
# probably a few more things.

Set Theme "Aardvark Blue"
Output demo.gif

Hide
Type "chabeau -p poe"
Enter
Sleep 250ms
Show

Type "The meaning of life?"
Sleep 500ms
Enter
Sleep 4s

Type "Go deeper"
Alt+Enter
Type "I will retry until you say something interesting. :-)"
Sleep 500ms
Enter
Sleep 4s
Ctrl+R
Sleep 4s
Ctrl+R
Sleep 4s
Ctrl+R


# Admire the output for a bit.
Sleep 4s

