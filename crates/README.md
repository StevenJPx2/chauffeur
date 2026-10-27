# Crates

The Rust engine. Each crate depends only on the ones below it in this table.

| Crate | Holds |
|---|---|
| `cli` | The `chauffeur` binary: `daemon`, `version`, `emit`, `events`, `status` |
| `daemon` | The private Unix socket, JSON-RPC 2.0 lines, loading and reloading configuration, routing events to rules, cooldowns |
| `runtime` | Running one rule for one event: redaction, a worker thread with a deadline, a bounded log |
| `protocol` | Rule files and their checks, the `when` filter, `then` entries, harness events and their evidence |
| `judges` | Questions, confidence, `models.json`, tiers in fallback order, and the `systemone` and `placeholder` formats |
| `core` | Plans of named steps, judges by name, events and the verdict. No dependencies |

A module with several parts is a folder named after it, one file per
concept, with a `README.md`:

| Folder | Holds |
|---|---|
| [`core/src/plan/`](core/src/plan/) | Plans, steps, outcomes and failures |
| [`judges/src/question/`](judges/src/question/) | Questions and their outcomes |
| [`judges/src/confidence/`](judges/src/confidence/) | Probabilities and how an answer settles |
| [`judges/src/config/`](judges/src/config/) | `models.json` |
| [`judges/src/placeholder/`](judges/src/placeholder/) | Recorded and fixed answers |
| [`judges/src/tiers/`](judges/src/tiers/) | `system_one/`, `system_two/`, `facts/`, `person/`, and fallback across a tier's models |
| [`protocol/src/rule/`](protocol/src/rule/) | Loading and checking a rule file |
| [`runtime/src/redact/`](runtime/src/redact/) | Redacting evidence |
| [`daemon/src/route/`](daemon/src/route/) | Routing events to rules, cooldowns, the log |

## One event

```mermaid
sequenceDiagram
    participant C as Caller
    participant D as daemon
    participant R as runtime
    participant J as judges
    C->>D: chauffeur.emit tool.after
    D->>D: rules whose on and when match, not cooling down
    D->>R: run each, at once, within the deadline
    R->>R: redact the event's evidence
    R->>J: the rule's plan, through core
    J-->>R: answers settled by confidence, or failures
    R-->>D: verdict, and the first then entry it meets
    D-->>C: actions, and a log entry per rule
```

```sh
echo '{"event":"tool.after","session":"s1","tool":"shell","input":{"command":"git commit -m wip"}}' | chauffeur emit
# → { "actions": [{ "rule": "conventional-commits", "action": "message.append", "payload": { … } }] }
chauffeur events
# → { "runs": [{ "rule": "conventional-commits", "steps": [{ "step": "nonconventional",
#      "answer": { "outcome": "yes", "p": { "yes": 0.93, "no": 0.07 }, "model": "jev", … } }] }] }
```

## Rules for every crate

- No `unwrap`, `expect` or panics outside tests; failures are values.
- A judge's failure is never an answer: it stays `failed`, whatever the confidence.
- Every queue, log, line and connection count has a bound.
- Nothing here embeds shipped content; it loads from `distribution/`.
