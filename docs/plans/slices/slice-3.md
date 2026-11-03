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

## What it builds

### `after` and rounds in core

A step's `after` names the earlier steps it waits for and the outcomes it
needs. Core validates a plan before any judge is asked, and drives it through
the round API: `questions`, `answer` and `fail`.

Files: `crates/core/src/after.rs`, `state.rs`, `proofs.rs`,
`crates/core/tests/scenarios.rs`

Verified by scenario tests:

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

And by Kani harnesses, each one invariant over at most three steps and a
small outcome enum. The `after` rule and round state are written over step
indices and outcome enums, with no strings or maps, so CBMC explores them in
seconds; the names and events around them stay in the code that calls them:

- `a_step_waits_until_every_need_has_finished`: no decision while a named step
  is `Pending` or `Asked`.
- `a_decision_follows_the_rule`: `Run` exactly when every `all` need and, if
  any, one `any` need is met.
- `a_skipped_step_meets_only_needs_without_when`: skipped never matches a `when`.
- `one_pass_settles_every_decidable_step`: skips propagate within one pass.
- `a_pass_only_moves_pending_steps_forward`: each step is asked at most once.
- `a_pass_with_no_open_round_asks_or_finishes`: a run never stalls, so it
  finishes within one round per step.

### Lowering rules to plans

Named judges, asks and `after` in a rule file become a `chauffeur_core::Plan`.

Files: `crates/protocol/src/lower.rs`

Verified by:

- Kani: `strategy_show_bounds_every_lowered_plan`, a lowered plan has at most
  the rounds `strategy show` reports.

### SubQ, GLiDE and richer questions

`choice` with every outcome's probability, and GLiDE's level-valued `score`,
in the `systemone` format; SubQ and GLiDE shipped beside Jev.

Files: `crates/judges/src/tiers/system_one/`, `distribution/models/subq.json`,
`glide.json`

Verified by:

- `main`'s `judges/{subq,glide}/tests/wire.rs` replayed over loopback.
- Labelled scale questions through `choice` and `score` report which agrees
  with the labels more often.

### Image evidence

Evidence blocks are text or images. An image goes only to a format that
accepts images, and only when a step names it.

Files: `crates/judges/src/evidence/`

Verified by:

- neo's screen check, with a recorded screenshot, reaches Jev with its opt-in
  and reaches no text-only format.

### The `student` format

jev-distill's students over its loopback server: the task's labels are the
outcomes, and evidence blocks fill its `state`.

Files: `crates/judges/src/tiers/system_one/student.rs`

Verified by:

- A question whose outcomes differ from the task's labels fails as `invalid`
  with no call; a student's `unsure` hands the step to Jev.

### A rest for a failing model

A model that fails rests for 60 seconds, then is tried again.

Files: `crates/judges/src/tiers/fallback.rs`

Verified by:

- A failing first model is skipped until its rest is over, then asked again.

### `chauffeur strategy`

`strategy list`: each loaded rule with its event, steps, and most rounds and
model calls. `strategy show <rule>` and `--file`: the tree, rounds, resolved
models, `after`, confidence, and the maximum rounds and model calls.

Files: `crates/cli/src/strategy.rs`

Verified by:

- `"model": "glide"` without GLiDE configured shows the tier's fallback
  order.

### The playground

The Strategy tab steps through the same tree.

Files: `playground/src/features/strategy.ts`

Verified by:

- The tab steps through each shipped rule's tree in a real browser.

## End to end

- `"model": "glide"` without GLiDE configured runs on the tier's fallback
  order and reports it.
- The escalation runs live against Jev and GLiDE and agrees with `main`'s
  `CHAUFFEUR_JUDGE_ESCALATE=glide` on the same cases, reporting answers,
  rounds, elapsed time and model calls.
