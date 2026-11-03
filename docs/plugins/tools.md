# `tools`

Keeps the prompt lean: hides the tool groups a task will not use, brings one
back when a later message needs it, points the agent at the Code Mode
namespaces a request needs, and shortens the longest built-in tool
descriptions. Slice 6.

| Hook                                                   | Does                                                                                                                                    |
| ------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------- |
| Start                                                  | Replaces built-in descriptions from `distribution/tools/descriptions.json`, through the tool registry                                   |
| `ctx.before("prompt.sent")`, a context's first message | One yes/no per tool group outside `base`; hides those whose `no` reaches 0.7                                                            |
| `ctx.before("prompt.sent")`, later messages            | One yes/no per hidden group; reveals a group the request now needs, saying that this re-reads the conversation once                     |
| `ctx.before("prompt.sent")`, with Code Mode            | One yes/no per namespace not yet surfaced, naming its best matches; a note with `search({ namespace })` goes out as a text part, `next` |
| `ctx.on("tool.after")`, a tool reported missing        | A `choice` of a hidden tool, then a yes/no with `after`; reveals it                                                                     |

A tool group is the ID before the first `_`, such as `artifact` for
`artifact_publish`. The hidden list goes to the harness with
`session.update`, and the adapter removes those tools from every request.
What it surfaced and revealed is kept per session in the plugin's storage.

## Commands

```sh
# Each tool group: shown, hidden or never exposed.
chauffeur tools list --session ses_42
chauffeur tools reveal artifact --session ses_42
```

```text
$ chauffeur tools list --session ses_42
GROUP      STATE    SINCE
artifact   hidden   first message
browser    shown    revealed at message 3
skill      never    never_exposed
```

`reveal` shows a group from the session's next step, as an `ask_chauffeur`
grant does.

## Options

```jsonc
{
  "tools": {
    "options": {
      "base": [
        "read",
        "edit",
        "patch",
        "write",
        "shell",
        "grep",
        "glob",
        "question",
        "subagent",
        "webfetch",
        "websearch",
        "history_search",
        "agents_spawn",
        "agents_send",
        "agents_read",
        "agents_list",
        "agents_stop"
      ],
      "never_exposed": ["skill"],
      "hide": { "no": 0.7 },
      "reveal": { "yes": 0.7 },
      "code_mode": {
        "trim": false,
        "always": ["context7", "jina", "opencode", "agents"]
      }
    }
  }
}
```

With `code_mode.trim`, the same answers also withhold the namespaces
confidently not needed, except those in `always`.

The harness package needs `session.update` to carry a session's hidden
tools, in every adapter that can remove tools from a request.
