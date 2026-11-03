# sourcefed

sourcefed watches GitHub pull requests, Jira issues and Slack threads and
delivers their events to agent sessions. It brings two things to Chauffeur,
both kept and installed by sourcefed:

| Piece                                                        | What it does                                                               | Slice |
| ------------------------------------------------------------ | -------------------------------------------------------------------------- | ----- |
| The `sourcefed/triage` rule                                  | Decides which events reach the agent, for every host sourcefed delivers to | 2     |
| The `sourcefed` plugin, in sourcefed's `adapters/chauffeur/` | Has sourcefed follow the session's own work                                | 5     |

## Triage

The rule is in the plan's [event triage](../plans/pluggability.md#event-triage);
the flow is in
[sourcefed: one triage for every host](../plans/pluggability.md#sourcefed-one-triage-for-every-host).

sourcefed's daemon asks before it queues each event for a target, over one
connection it keeps for its lifetime:

```ts
import { connect } from "@fdcn/chauffeur/client"

const chauffeur = connect() // at startup; chauffeur.close() at shutdown

const { intents } = await chauffeur.navigate(
  { on: "sourcefed.event", evidence: { event, thread, monitor } },
  { signal: AbortSignal.timeout(3_000) }
)
if (!intents.some((intent) => intent.withhold === "event")) queue(target, event)
```

With no Chauffeur, an error, or no answer within 3 seconds, sourcefed
delivers.

## The plugin

The `sourcefed` plugin follows the session's own work: the pull request the
agent opened, the Jira issue it is working on, the Slack thread it is in. New
reviews, CI results and comments then reach the agent. It lives in sourcefed's
repository, beside the daemon whose RPC it calls, and sourcefed's installer
adds it to `~/.config/chauffeur/plugins.json`. Its ID gives it the
`sourcefed.*` events, which no other plugin can send.

| Hook                   | Does                                                                                                        |
| ---------------------- | ----------------------------------------------------------------------------------------------------------- |
| `ctx.on("tool.after")` | Finds candidates in the call and its result: a PR link from `gh pr create`, a Jira key, a Slack thread link |

For each candidate no monitor watches yet, once per session, it asks:

| Question                                                          | Confidence | `yes`                                                                                                                              |
| ----------------------------------------------------------------- | ---------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| "Is this the session's own work, which it should keep following?" | 0.7        | sourcefed's RPC creates the monitor, then `message.append` `now`: "sourcefed now watches …; do not create another monitor for it." |

An unreachable sourcefed daemon, a `no`, `unsure` or a failure creates
nothing. The candidates already judged are kept in the plugin's storage, per
session.

```json
{
  "sourcefed": {
    "path": "~/.local/share/sourcefed/chauffeur",
    "options": {
      "daemon": {
        "url": "http://127.0.0.1:18787",
        "token": { "env": "SOURCEFED_DAEMON_TOKEN" }
      },
      "follow": { "confidence": 0.7 }
    }
  }
}
```

## What it needs

| Piece                                                       | Slice |
| ----------------------------------------------------------- | ----- |
| `navigate`, intents and the TypeScript client               | 2     |
| Plugins, and `plugins.json` entries an integration installs | 5     |
