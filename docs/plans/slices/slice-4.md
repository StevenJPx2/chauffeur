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

Verified by:

- A fact over its memory cap or deadline fails the step.
- A project fact cannot import a host module.

### The `facts` format

The facts tier: a step whose `judge` is `facts` runs a fact.

Files: `crates/judges/src/tiers/facts/`

Verified by:

- A missing fact falls back to a placeholder.

### Shipped facts

`match` and `select`, with their tests.

Files: `distribution/facts/match.js`, `select.js`

Verified by:

- `match` passes v1's facts-judge cases.
- `select` passes exhaustive tests over every input up to 4 candidates.

### Candidates and `for_each`

Candidates, `for_each` copies, wildcards and `[item]` in lowering.

Files: `crates/protocol/src/lower.rs`

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
