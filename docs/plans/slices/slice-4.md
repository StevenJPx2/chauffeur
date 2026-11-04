# Slice 4: opencode-history recall

**Status:** Planned.

**You can:** have opencode-history attach the right past passages when you
send a message, as in [Per-turn recall](../pluggability.md#per-turn-recall): its rule installed
in `~/.config/chauffeur/rules/opencode-history/`, its code reduced to one
`navigate` call. You can also write your own facts.

This slice adds facts, `for_each` over candidates, and `select`.

## What it builds

### The facts sandbox

Fact modules from the three sources, run in QuickJS with memory caps and
deadline interrupts, with read-only host modules for shipped and user facts.

Files: `crates/facts/src/lib.rs`, `sandbox.rs`, `host.rs`, `limits.rs`

```js
// ~/.config/chauffeur/facts/notes.js: a user fact, with a read-only host module
import { query } from "chauffeur:sqlite"

export default ({ evidence, args }) => {
  const rows = query(
    "~/notes/index.db",
    "SELECT id FROM notes WHERE notes MATCH ? LIMIT ?",
    [evidence.message, args.top ?? 10]
  )

  return { ranked: rows.map((row) => row.id) }
}
```

Verified by:

- A fact over its memory cap or deadline fails the step.
- A project fact cannot import a host module.

### The `facts` format

The facts tier: a step whose `judge` is `facts` runs a fact.

Files: `crates/judges/src/tiers/facts/`

```json
{
  "id": "is-slack",
  "judge": "facts",
  "ask": { "match": { "field": "event.source", "equals": "slack" } }
}
```

Verified by:

- A missing fact falls back to a placeholder.

### Shipped facts

`match` and `select`, with their tests.

Files: `distribution/facts/match.js`, `select.js`

```json
{
  "id": "best",
  "judge": "facts",
  "after": ["helps[*]"],
  "ask": { "select": { "from": "helps[*]", "top": 3, "min": 0.7, "by": "yes" } }
}
```

Verified by:

- `match` passes v1's facts-judge cases.
- `select` passes exhaustive tests over every input up to 4 candidates.

### Candidates and `for_each`

Candidates, `for_each` copies, wildcards and `[item]` in lowering.

Files: `crates/protocol/src/lower.rs`

```json
{
  "id": "helps",
  "judge": "system-one",
  "for_each": "candidates",
  "ask": {
    "outcomes": ["yes", "no"],
    "evidence": ["message", "candidate"],
    "instructions": "Would this passage help the agent answer the message?"
  }
}
```

One copy of the step per candidate, all in one round, so Jev answers them in
one call.

Verified by:

- Each row of the [strategies table](../pluggability.md#strategies) runs as a
  rule with recorded answers.
- `after` with `[item]` skips exactly the candidates whose earlier copy did not
  match.

### The playground

The Facts tab.

Files: `playground/src/features/facts.ts`

Verified by:

- The tab runs `match` and `select` in a real browser.

## End to end

- Jev scores recorded by opencode-history's replay feed a placeholder System
  One, and navigating `opencode-history.recall` attaches the replay's top
  three passages scoring 0.7 or above.
- Live, it makes one Jev call per message, with judge time within the
  replay's 278 ms p50 and 318 ms p95 plus local overhead. No agent session or
  history is sent.
