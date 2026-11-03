# Tools

v1's tool exposure kept the prompt lean: it hid the tool groups a task would
not use, brought them back when a later message needed them, pointed the
agent at the Code Mode namespaces a request needed, and shortened the longest
built-in tool descriptions. In v2 this is the `tools` plugin, in slice 6.

| v1                                                                                                                     | v2                                                                                                    |
| ---------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| At a context's first message, one yes/no per tool group outside `base`; hide at P ≤ 0.3                                | `ctx.before("prompt.sent")` on the first message: `for_each` group, hide those whose `no` reaches 0.7 |
| `base` and `never_exposed`                                                                                             | The same lists, as options                                                                            |
| Reveal on a later message at P ≥ 0.7, noting that it re-reads the conversation                                         | The same question on later messages; the plugin records what it revealed                              |
| After a tool result reporting a missing tool: pick a hidden tool, then confirm it                                      | `ctx.on("tool.after")`, a `choice` then a yes/no with `after`                                         |
| Code Mode notes: per namespace not yet surfaced, a yes/no naming its best matches; a note with `search({ namespace })` | The same strategy; the note is a `message.append` text part with `next` delivery                      |
| Code Mode trimming (off)                                                                                               | The same option, off                                                                                  |
| Shorter built-in tool descriptions                                                                                     | The plugin replaces them through the tool registry, from `distribution/tools/descriptions.json`       |
| Hidden tools recorded on message metadata and removed in the `context` hook                                            | `session.update` with the hidden list; the adapter removes those tools from each request              |

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

The harness package needs `session.update` to carry a session's hidden tools,
in every adapter that can remove tools from a request.
