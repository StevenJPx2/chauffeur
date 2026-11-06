# `chauffeur emit`

Sends one harness event, read as JSON on stdin, to the daemon, and prints
what the rules returned. Chauffeur's adapters send these events themselves; `emit` is for trying rules and plugins by hand.

```sh
echo '{ "event": "tool.after", "session": "s1", "tool": "shell",
        "input": { "command": "git commit -m '"'"'fixed stuff'"'"'" } }' \
  | chauffeur emit
```

```json
{
  "actions": [
    {
      "rule": "conventional-commits",
      "action": "message.append",
      "payload": {
        "label": "commit message isn't conventional; asking the agent to amend it",
        "parts": [
          { "text": "Chauffeur, which the user set up to check commits: …" }
        ],
        "delivery": "now"
      }
    }
  ]
}
```

| Option            | Default                 |
| ----------------- | ----------------------- |
| `--deadline-ms N` | `3000`; at most `30000` |

- The event is a JSON object whose `event` names it. Its other fields are
  what rules filter and judge on, such as `session`, `tool` and `input`.
- `actions` is empty when no rule matched, a rule was cooling down, a judge
  failed, or nothing in a rule's `then` held. [`audit`](audit.md) says which.
- Errors: stdin that is not JSON, no daemon on the socket, or an event the
  daemon refuses, each with its message.
