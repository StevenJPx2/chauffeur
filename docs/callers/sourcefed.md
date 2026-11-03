# sourcefed

sourcefed watches GitHub pull requests, Jira issues and Slack threads and
delivers their events to agent sessions. Chauffeur decides which events reach
the agent, for every host sourcefed delivers to.

The rule is `sourcefed/triage`, in the plan's
[event triage](../plans/pluggability.md#event-triage); the flow is in
[sourcefed: one triage for every host](../plans/pluggability.md#sourcefed-one-triage-for-every-host).

## The call

sourcefed's daemon asks before it queues an event for a target:

```ts
const { intents } = await connect().navigate(
  { on: "integration_event", evidence: { event, thread, monitor } },
  { signal: AbortSignal.timeout(3_000) }
)
if (!intents.some((intent) => intent.withhold === "event")) queue(target, event)
```

With no Chauffeur, an error, or no answer within 3 seconds, sourcefed
delivers.

The `monitors` plugin, in slice 5, starts sourcefed monitors for the
session's own work; see [sourcefed events and monitors](../migration/sourcefed.md).

## What it needs

| Piece                                         | Slice |
| --------------------------------------------- | ----- |
| `navigate`, intents and the TypeScript client | 2     |
