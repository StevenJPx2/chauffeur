# Slice 4: opencode-history recall

**Status:** Planned.

**You can:** have opencode-history attach the right past passages when you
send a message, as in [Per-turn recall](../pluggability.md#per-turn-recall): its rule installed
in `~/.config/chauffeur/rules/opencode-history/`, its code reduced to one
`navigate` call. You can also write your own facts.

This slice adds facts, `for_each` over candidates, and `select`.

| File                                                   | Contents                                                                                                                                     |
| ------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/facts/src/{lib,sandbox,host,limits}.rs`        | Fact modules from the three sources; QuickJS with memory caps and deadline interrupts; the read-only host modules for shipped and user facts |
| `crates/judges/src/tiers/facts/`                       | The `facts` format                                                                                                                           |
| `distribution/facts/{match,select}.js` and their tests | The two shipped facts                                                                                                                        |
| `crates/protocol/src/lower.rs`                         | Candidates, `for_each` copies, wildcards and `[item]`                                                                                        |
| `playground/src/features/facts.ts`                     | The Facts tab                                                                                                                                |

Verification:

- Facts: `match` passes v1's facts-judge cases; `select` passes exhaustive
  tests over every input up to 4 candidates; a fact over its memory cap or
  deadline fails the step; a project fact cannot import a host module; a
  missing fact falls back to a placeholder.
- Strategies: each row of the [strategies table](../pluggability.md#strategies) runs as a rule
  with recorded answers; `after` with `[item]` skips exactly the candidates
  whose earlier copy did not match.
- Recall: Jev scores recorded by opencode-history's replay feed a placeholder
  System One, and navigating `opencode-history.recall` attaches the replay's top three passages
  scoring 0.7 or above. Live, it makes one Jev call per message, with judge
  time within the replay's 278 ms p50 and 318 ms p95 plus local overhead. No
  agent session or history is sent.
