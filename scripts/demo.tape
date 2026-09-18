# The README demo: a real clank session, recorded by scripts/record-demo.sh.
#
# This is a VHS tape (https://github.com/charmbracelet/vhs) - VHS drives a real
# terminal through ttyd and captures the frames, so what the GIF shows is what
# the commands did, including the model's latency and the exit codes.
#
# Run it through the wrapper, not directly: the wrapper resolves the binary, the
# endpoint and the model, warms the model, and gates on the result.
#
#   scripts/record-demo.sh
#
# Two rules make the recording readable. Every beat is one shell command, with
# its gate (`printf '\n[gate N: exit %d]' $?`) on the same command line, so
# nothing is typed while clank is still answering - a typed-ahead gate echoes on
# top of the answer it is supposed to come after. And the gate text is the wait
# target, so VHS types the next beat only once this one has finished; the exit
# status of every beat is then visible in the recording rather than implied.
#
# The failure beat is a single line for a third reason: split across a `\`
# continuation with the redirect starting the second line, the shell ended up
# sitting at its continuation prompt - the truncation reason was on screen, the
# gate never printed, and the wait timed out after the full `Set WaitTimeout`.
# One line per beat where it fits (139 columns at this width and font) avoids it.

Output docs/images/clank-demo.gif

Set Shell bash
Set Theme "Catppuccin Mocha"
Set FontFamily "JetBrainsMono Nerd Font"
Set FontSize 14
Set Width 1180
Set Height 700
Set Padding 20
Set TypingSpeed 25ms
Set Framerate 30
Set PlaybackSpeed 2
Set WaitTimeout 300s
Set CursorBlink false

# Preamble: hidden, so the first visible line is the provenance of the run.
Hide
Type `export PS1='$ '`
Enter
Sleep 300ms
Type `clear`
Enter
Sleep 300ms
Type `printf 'model %s @ %s\n' "$CLANK_MODEL" "$CLANK_BASE_URL"`
Enter
Sleep 400ms
Show

# 1. The capability surface: four read-only observers, no shell, no writes.
Type `clank --list-tools | jq -r '.[].function.name'; printf '\n[gate 1: exit %d]\n' $?`
Enter
Wait+Screen /\[gate 1: exit [0-9]\]/

# 2. The pipe is the context: the shell gathers the evidence, clank reads it.
Type `rg -n userData fixtures | clank --thinking off --max-tokens 90 \`
Enter
Type `    -m 'what does this do?'; printf '\n[gate 2: exit %d]\n' $?`
Enter
Wait+Screen /\[gate 2: exit [0-9]\]/

# 3. The map: one prompt per item, framed so stdout maps back to stdin.
Type `printf 'build green\ntwo tests failed\ndeploy paused\n' | clank --each \`
Enter
Type `    --thinking off -m 'good news or bad? one line'; printf '\n[gate 3: exit %d]\n' $?`
Enter
Wait+Screen /\[gate 3: exit [0-9]\]/

# 4. Structured output: the schema is enforced by the server, jq is the gate.
Type `echo 'the build is green' | clank --thinking off \`
Enter
Type `    --json-schema @fixtures/verdict.schema.json -m 'verdict' | jq -c .; \`
Enter
Type `    printf '\n[gate 4: exit %d]\n' $?`
Enter
Wait+Screen /\[gate 4: exit [0-9]\]/

# 5. Failure is loud and it has an exit code - the whole contract, in one line.
#    stdout is discarded so the recording shows the reason, not a stub answer.
Type `clank --thinking off --max-tokens 4 -m 'write a paragraph about socks' >/dev/null; printf '\n[gate 5: exit %d]\n' $?`
Enter
Wait+Screen /\[gate 5: exit [0-9]\]/
