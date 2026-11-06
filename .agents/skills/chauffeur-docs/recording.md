# Recording a slice

Slice 1's recording, as commands to adapt for the next slice. It ends with
`docs/media/slice-1.mp4`: OpenCode above the audit log, 17 seconds, in which
the agent commits, Chauffeur's message arrives and the agent amends.

## Set up

A scratch repo whose OpenCode configuration loads only the rc's adapter, and
the rc's daemon. It leaves out v1's plugin and `opencode-history`, whose recall
carries what earlier runs learned into the next one. Its first commit is not
conventional, so the agent has no history to copy. Use `brew --prefix` in place
of `/opt/homebrew`.

```sh
T=$TMPDIR/opencode/rec-slice-1
mkdir -p $T/repo && cd $T/repo && git init -q

printf '# Greeter\n\nA tiny project for trying Chauffeur.\n' > README.md
cat > opencode.jsonc <<'EOF'
{
  "$schema": "https://opencode.ai/config.json",
  "plugins": [
    "-chauffeur",
    "-opencode-history",
    "/opt/homebrew/share/chauffeur-next/opencode"
  ]
}
EOF
git add -A && git commit -qm "Initial commit"

pgrep -f 'chauffeur-next daemon' \
  || (chauffeur-next daemon run > $T/daemon.log 2>&1 &)
```

## Record

The audit log first, so it is ready before anything happens; then OpenCode,
without tabs. Mark both sessions at each moment.

```sh
termctrl start audit --record $T/audit.termctrl --cols 120 --rows 6 \
  --cwd $T/repo -- chauffeur-next audit 0 --follow --brief

termctrl start oc --record $T/oc.termctrl --host opentui --cols 120 --rows 32 \
  --cwd $T/repo -- env 'OPENCODE_CLI_CONFIG_CONTENT={"tabs":{"mode":"off"}}' \
  opencode --standalone

termctrl wait oc "Ask anything" --timeout 60000
termctrl wait oc "Build ·" --timeout 30000
mark() { termctrl mark oc $1; termctrl mark audit $1; }

mark ready
termctrl send oc --pace-ms 30 'text:Add a line saying hello to README.md and commit it.'
mark typed

termctrl send oc enter
termctrl wait audit "message.append" --timeout 120000
mark nudged

termctrl wait oc "tok/s" --timeout 180000
mark answered

termctrl stop oc
termctrl stop audit
```

`termctrl wait` reads the screen, so the text must fit on one row: at fewer
than 120 columns the audit line wraps and the wait misses `message.append`.

The agent writes its own message, so a run may commit conventionally and get
no nudge: the audit line then ends without `→ message.append`. Reset the repo
to its first commit and record again, until a take shows the nudge and the
agent's answer to it.

## Export

One clip list for both panes, so they stay in step. `oc.json`:

```json
{
  "clips": [
    {
      "from": "ready",
      "to": "typed",
      "speed": 1,
      "caption": "OpenCode is asked to add a line to the README and commit it"
    },
    {
      "from": "typed",
      "to": "nudged",
      "speed": 1.5,
      "caption": "The agent commits as \"Add …\"; the adapter sends tool.after to Chauffeur"
    },
    {
      "from": "nudged",
      "to": "answered",
      "speed": 1,
      "caption": "Jev says it breaks Conventional Commits; Chauffeur's message arrives and the agent amends",
      "hold_ms": 6000
    }
  ]
}
```

`audit.json` is the same with every caption
`chauffeur-next audit --follow --brief`.

```sh
cd $T
termctrl video oc.termctrl --edit oc.json --tail-ms 0 --hide-cursor -o oc.mp4
termctrl video audit.termctrl --edit audit.json --tail-ms 0 --hide-cursor \
  --include-startup -o audit.mp4

ffmpeg -y -i oc.mp4 -i audit.mp4 -filter_complex \
  "[0:v][1:v]vstack=inputs=2:shortest=1,scale=1600:-2:flags=lanczos,format=yuv420p" \
  -c:v libx264 -preset slow -crf 26 -movflags +faststart -an slice-1.mp4

cp slice-1.mp4 ~/path/to/chauffeur/docs/media/slice-1.mp4
```

Render in the scratch folder and copy only the finished video into the
repository.

## Check and clean up

Look at a frame from each clip, such as `ffmpeg -ss 8 -i … -frames:v 1 f.png`:
the audit pane is empty until the nudge, and no frame shows a key or another
project. Then delete the test sessions, which live only in the scratch
directory:

```sh
opencode api get "/api/session?directory=$T/repo" \
  | jq -r --arg d "$T/repo" '.data[] | select(.location.directory == $d) | .id' \
  | while read -r s; do opencode api delete /api/session/$s; done
```

Stop the daemon if it was not running before, and remove `$T`.
