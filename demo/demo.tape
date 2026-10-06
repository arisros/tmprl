# Records the README's demo against the sample cluster. From the repository root:
#
#   demo/run.sh up && demo/run.sh seed 300
#   vhs demo/demo.tape
#
# Nothing in it is anyone's data: the cluster is local and every value is invented.

Output docs/img/demo.gif

Set Shell "bash"
Set FontSize 15
Set Width 1720
Set Height 980
Set Padding 12
Set TypingSpeed 70ms
Set PlaybackSpeed 1.0
Env EDITOR "nvim --clean"
Env PS1 "$ "

Type "demo/run.sh tmprl"
Sleep 600ms
Enter
Sleep 2s

# The namespace, then its workflows, narrowed to the ones that failed.
Enter
Sleep 2500ms
Type "i"
Sleep 400ms
Type "ExecutionStatus = 'Failed'"
Sleep 500ms
Enter
Sleep 2500ms

# One of them: jump to what failed, read why, with its payload decoded.
Enter
Sleep 2500ms
Type "]f"
Sleep 1500ms
Type "K"
Sleep 3s

# Go to the code behind it.
Type "gf"
Sleep 4s
Type "jjj"
Sleep 1200ms
Type "ZQ"
Sleep 1500ms

# The same history as a timeline.
Space
Sleep 300ms
Type "G"
Sleep 3500ms

# Up a level, and to the dashboard: counts, charts, tallies, failures, and what keeps
# being retried.
Type "-"
Sleep 1200ms
Type "gd"
Sleep 4500ms
Type "j"
Sleep 900ms
Type "l"
Sleep 900ms
Type "G"
Sleep 2500ms

# Open one that is retrying, and go to the activity that keeps failing: the attempt it is
# on, the last failure, when the next try is due.
Enter
Sleep 2500ms
Type "G"
Sleep 1200ms
Type "K"
Sleep 4s
