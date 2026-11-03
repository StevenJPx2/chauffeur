# sourcefed events and monitors

v1 decided which sourcefed events reach the agent, and started sourcefed
monitors for the PR, Jira issue or Slack thread the agent was working on.
In v2 the first is the `sourcefed/triage` rule that sourcefed calls through
`navigate`, in slice 2, and the second is the `monitors` plugin, in slice 5
with the other first plugins.

## The event gate

| v1                                                                         | v2                                                                                                         |
| -------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| sourcefed's OpenCode plugin calls the adapter's `chauffeur.gate` RPC       | sourcefed's daemon navigates `integration_event`, for every host; see [sourcefed](../callers/sourcefed.md) |
| "Should the agent be shown this event now?", told what the monitor watches | The `needs-agent` question, with `event`, `thread` and `monitor` evidence                                  |
| Withhold only on a confident no (P ≤ 0.3); everything else delivers        | `confidence: 0.7` and `then` withholds on `no`                                                             |
| `withhold_bodies`: `[message without text]` withheld without asking        | A `match` fact step on `event.body`, from slice 4; until then the screen judges it                         |
| `POST /integrations/sourcefed` for other integrations                      | `navigate` with the same event, from any program                                                           |
| sourcefed delivers when Chauffeur fails or takes over 3 s                  | The same                                                                                                   |

v1's question, abridged, becomes the rule's `needs-agent` ask:

```json
{
  "on": "integration_event",
  "judges": { "quick": { "tier": "system-one", "confidence": 0.7 } },
  "strategy": [
    {
      "id": "screen",
      "judge": "quick",
      "ask": {
        "outcomes": ["yes", "no"],
        "evidence": ["event", "thread", "monitor"],
        "instructions": "Should the agent be shown this event now? Show events that need the agent to do something: … Withhold noise: …"
      }
    }
  ],
  "then": [{ "if": { "screen": "no" }, "withhold": "event" }]
}
```

## Monitors

| v1                                                                                                               | v2                                                                      |
| ---------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- |
| After a tool result, exact facts name candidates: a PR link from `gh pr create`, a Jira key, a Slack thread link | `ctx.on("tool.after")` in the `monitors` plugin, with the same patterns |
| A candidate a monitor already watches is dropped                                                                 | The plugin asks sourcefed's daemon first                                |
| "Is this the session's own work to keep following?" at P ≥ 0.7                                                   | The same yes/no, `confidence: 0.7`                                      |
| Creates the monitor, then steers: "sourcefed now watches …"                                                      | sourcefed's RPC, then `message.append` with `now`                       |
| Each candidate judged once per agent; unreachable sourcefed creates nothing                                      | The same, in the plugin's state                                         |
| `SOURCEFED_DAEMON_URL`, `SOURCEFED_DAEMON_TOKEN`, `CHAUFFEUR_SOURCEFED=off`                                      | The plugin's options; leaving it out of `plugins.json` turns it off     |
