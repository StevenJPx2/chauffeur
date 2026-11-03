# Slice 3: Escalation from Jev to GLiDE

**Status:** Planned.

**You can:** have the commit nudge and `sourcefed/triage` ask GLiDE only when
Jev is unsure or fails, see each strategy as a tree with
`chauffeur strategy show`, and step through it in the playground's Strategy
tab. neo checks an iPhone screen through Chauffeur with its screenshot, and a
jev-distill student answers before Jev.

This slice adds strategies with more than one step: a step's `after` names
the earlier steps it waits for and the outcomes it needs, as in the
[core plan model](../pluggability.md#core-plan-model). It also adds image evidence, the
`student` format, and a tier that rests a failed model for 60 seconds before
asking it again.

| File                                                          | Contents                                                                                                                                                                                                                                          |
| ------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/core/src/{after,state}.rs`                            | The `after` rule, validation, and the round API (`questions`, `answer`, `fail`)                                                                                                                                                                   |
| `crates/core/src/proofs.rs`, `crates/core/tests/scenarios.rs` | Kani harnesses and scenario tests                                                                                                                                                                                                                 |
| `crates/protocol/src/lower.rs`                                | Lowering named judges, asks and `after` to `chauffeur_core::Plan`                                                                                                                                                                                 |
| `crates/judges/src/tiers/system_one/`                         | `choice` with every outcome's probability, and GLiDE's level-valued `score`                                                                                                                                                                       |
| `distribution/models/{subq,glide}.json`                       | SubQ and GLiDE beside Jev                                                                                                                                                                                                                         |
| `crates/judges/src/evidence/`                                 | Text and image blocks; an image goes only to a format that accepts images, and only when a step names it                                                                                                                                          |
| `crates/judges/src/tiers/system_one/student.rs`               | The `student` format for jev-distill's loopback server                                                                                                                                                                                            |
| `crates/judges/src/tiers/fallback.rs`                         | A model that fails rests for 60 seconds, then is tried again                                                                                                                                                                                      |
| `crates/cli/src/strategy.rs`                                  | `chauffeur strategy list`: each loaded rule with its event, steps, and most rounds and model calls; `chauffeur strategy show <rule>` and `--file`: the tree, rounds, resolved models, `after`, confidence, and the maximum rounds and model calls |
| `playground/src/features/strategy.ts`                         | The Strategy tab                                                                                                                                                                                                                                  |

Scenario tests:

- Escalation, with recorded answers at every tier.
- `after` alone: a string need waits for a skipped step and then runs; an
  object need never matches a skipped step; `Failed` matches only when listed;
  a step with both `all` and `any` runs only when both hold.
- Validation: an empty plan, a duplicate ID, `after` naming a later or unknown
  step, an empty `when` list, and a judge the panel lacks, each rejected
  before any judge is asked.
- The round API: a reply naming an unknown, unasked or repeated step, or
  missing one, returns the unchanged round; each judge answers its share of a
  round in one call; a batch of the wrong length settles as `Failed`.

Kani harnesses, each one invariant over at most three steps and a small
outcome enum. The `after` rule and round state are written over step indices
and outcome enums, with no strings or maps, so CBMC explores them in seconds;
the names and events around them stay in the code that calls them:

| Harness                                        | Property                                                               |
| ---------------------------------------------- | ---------------------------------------------------------------------- |
| `a_step_waits_until_every_need_has_finished`   | No decision while a named step is `Pending` or `Asked`                 |
| `a_decision_follows_the_rule`                  | `Run` exactly when every `all` need and, if any, one `any` need is met |
| `a_skipped_step_meets_only_needs_without_when` | Skipped never matches a `when`                                         |
| `one_pass_settles_every_decidable_step`        | Skips propagate within one pass                                        |
| `a_pass_only_moves_pending_steps_forward`      | Each step is asked at most once                                        |
| `a_pass_with_no_open_round_asks_or_finishes`   | A run never stalls, so it finishes within one round per step           |
| `strategy_show_bounds_every_lowered_plan`      | A lowered plan has at most the rounds `strategy show` reports          |

Verification: `main`'s `judges/{subq,glide}/tests/wire.rs` replayed over
loopback; `"model": "glide"` without GLiDE configured runs on the tier's
fallback order and reports it; the escalation runs live against Jev and GLiDE
and agrees with `main`'s `CHAUFFEUR_JUDGE_ESCALATE=glide` on the same cases,
reporting answers, rounds, elapsed time and model calls; labelled scale
questions through `choice` and `score` report which agrees with the labels
more often.
