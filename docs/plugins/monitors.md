# `monitors`

Has sourcefed follow the session's own work: the pull request the agent
opened, the Jira issue it is working on, the Slack thread it is in. New
reviews, CI results and comments then reach the agent. Slice 5.

| Hook                   | Does                                                                                                        |
| ---------------------- | ----------------------------------------------------------------------------------------------------------- |
| `ctx.on("tool.after")` | Finds candidates in the call and its result: a PR link from `gh pr create`, a Jira key, a Slack thread link |

For each candidate no monitor watches yet, once per session, it asks:

| Question                                                          | Confidence | `yes`                                                                                                                              |
| ----------------------------------------------------------------- | ---------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| "Is this the session's own work, which it should keep following?" | 0.7        | sourcefed's RPC creates the monitor, then `message.append` `now`: "sourcefed now watches …; do not create another monitor for it." |

An unreachable sourcefed, a `no`, `unsure` or a failure creates nothing.

## Options

```json
{
  "monitors": {
    "options": {
      "sourcefed": {
        "url": "http://127.0.0.1:18787",
        "token": { "env": "SOURCEFED_DAEMON_TOKEN" }
      },
      "follow": { "confidence": 0.7 }
    }
  }
}
```

The candidates already judged are kept in the plugin's storage, per session.
Filtering sourcefed's events is the `sourcefed/triage` rule, not this plugin:
see [sourcefed](../callers/sourcefed.md).
