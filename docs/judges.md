# Judges

A judge is whatever answers what a capability asks. Every judge fills the same
role, `chauffeur_core::Judge`: it is handed a batch of questions and
reasonings and returns a ruling for each. Three kinds ship:

| Judge        | What it is                                     | Answers                                        |
| ------------ | ---------------------------------------------- | ---------------------------------------------- |
| `system_one` | a fast hosted classifier: Jev or SubQ          | typed questions (yes/no, choice, score)        |
| `system_two` | a general-purpose LLM that reasons             | reasonings over evidence, to a schema          |
| `facts`      | plain code, no model                           | yes/no questions, from the facts they carry    |

A capability never knows which judge answered. Each question carries its
facts (named whole numbers, such as `idle_seconds`) beside its wording, so a
judge that reads facts and a model that reads text can both answer it.

## Routes

Every question goes to `system_one` and every reasoning to `system_two`,
unless `routes` in [`skills/config/judge.json`](../skills/config/judge.json)
names another judge for its ID (`capability/question`):

```json
{
  "routes": { "model-router/renew_context": "facts" },
  "facts": {
    "model-router/renew_context": {
      "all": [
        { "fact": "idle_seconds", "at_least": { "fact": "cache_seconds" } },
        { "fact": "context_tokens", "at_least": { "fact": "context_budget" } }
      ]
    }
  }
}
```

The model router computes `context_budget` from `renewal.budget` in
`model-router.json`: `share` of the model's window, clamped between
`min_tokens` and `max_tokens` (the ceiling when the window is unknown).
`cache_seconds` comes from `renewal.cache_seconds`, by provider.

- A `facts` rule holds when `all` (or `any`) of its conditions hold. A
  condition compares one fact `at_least` or `at_most` a number, or another
  fact scaled by `times`. A holding rule answers yes with certainty; otherwise
  no. A question with no rule, or missing a fact its rule reads, goes
  unanswered, and its capability applies its usual failure posture.
- Your `~/.config/chauffeur/judge.json` can add routes and rules or replace one
  (`{ "routes": { "model-router/renew_context": "system_one" } }` hands the
  question back to Jev). A route to an unknown judge, or to `facts` without a
  rule, stops the daemon with an error naming it. Edits apply without a restart.
- The engine runs the facts judge itself while it plans a round, with no I/O:
  a signal whose questions all go to it never waits on a model, and a System
  One failure does not lose its answers. System One and System Two run on the
  daemon's worker pools, as before.

## System One providers

Chauffeur supports three System One providers:

- **Jev**, from TypeSafe, the default (`jev-latest`).
- **SubQ Snap**, optional (`subq-snap-preview`).
- **Fastino GLiDE**, optional (`fastino/GLiDE`).

All receive the same locally redacted state and typed questions. They share
the `/v1/systemone` transport and answer validation; capability thresholds and
failure postures do not change when the provider changes.

## Selection

Set these in the environment that starts the daemon, then restart it. Exporting
variables in a terminal does not change an already running daemon.

Jev only, the default:

```sh
export TYPESAFE_API_KEY=...
export CHAUFFEUR_JUDGE=jev
export CHAUFFEUR_JUDGE_FALLBACK=none
```

Jev primary, SubQ fallback:

```sh
export TYPESAFE_API_KEY=...
export SUBQ_API_KEY=...
export CHAUFFEUR_JUDGE=jev
export CHAUFFEUR_JUDGE_FALLBACK=subq
```

SubQ primary, Jev fallback:

```sh
export TYPESAFE_API_KEY=...
export SUBQ_API_KEY=...
export CHAUFFEUR_JUDGE=subq
export CHAUFFEUR_JUDGE_FALLBACK=jev
```

For SubQ alone, select `subq` and set fallback to `none`; no TypeSafe key is
required. An unset or empty fallback also means none. A key alone never enables
a provider. Unknown names, identical primary and fallback, or missing credentials
for a selected provider stop startup with a configuration error.

GLiDE alone uses `CHAUFFEUR_JUDGE=glide` and `FASTINO_API_KEY`.
To keep Jev primary and use GLiDE only on provider failure, set
`CHAUFFEUR_JUDGE=jev` and `CHAUFFEUR_JUDGE_FALLBACK=glide` with both keys.
This is **not** confidence-based escalation: low-confidence valid answers
remain final. Merely setting `FASTINO_API_KEY` changes nothing.

Provider-specific overrides:

- `TYPESAFE_BASE_URL`, default `https://api.typesafe.ai`.
- `TYPESAFE_DEFAULT_MODEL`, default `jev-latest`.
- `SUBQ_BASE_URL`, default `https://api.subq.ai`.
- `SUBQ_DEFAULT_MODEL`, default `subq-snap-preview`.
- `FASTINO_BASE_URL`, default `https://api.fastino.ai`.
- `FASTINO_DEFAULT_MODEL`, default `fastino/GLiDE`.

GLiDE always sends `store: false` and accepts Bearer authentication. Its
integer `score` is the winning level, not Chauffeur's weighted score: the
client requires `expected_level` for Score answers and validates its range.
Missing or invalid weighted scores fail closed. Jev/SubQ parsing is unchanged.
The existing three-second deadline and single-attempt policy also apply to
GLiDE. Fastino recommends a much longer timeout and warm-up retries for general
clients; Chauffeur deliberately fails or uses its selected fallback instead of
waiting through a cold deployment inside an agent hook.

## Fallback behavior

- Try the primary first.
- On a transport error, HTTP error, malformed response, or invalid answers,
  try the fallback once with the same redacted state and questions.
- A valid low-confidence answer is final. Fallback is not a second opinion.
- After primary failure, use the fallback for 60 seconds. The next judgment
  after that window probes the primary; a success restores it.
- Both providers have a 3-second timeout. The first failing-primary call can
  take up to about 6 seconds; subsequent cooldown calls try only the fallback.
- There are at most two judgment rounds per engine signal. This stays within
  the daemon's 15-second reply timeout.
- If fallback also fails, return the failure to the engine. Permissions still
  fall back to asking the user, and other capabilities keep their existing
  failure behavior. No answer is manufactured.

The daemon logs the fallback transition. `CHAUFFEUR_RECORD_JUDGMENTS=PATH`
records each attempted provider separately, including `provider`, model,
request, result and latency. Credentials are not recorded. Treat recordings as
sensitive even though the engine redacts known credential shapes.

## Confidence escalation

Fallback answers failures; escalation answers doubt. It is off unless
`CHAUFFEUR_JUDGE_ESCALATE` names a second judge:

```sh
export TYPESAFE_API_KEY=...
export FASTINO_API_KEY=...
export CHAUFFEUR_JUDGE=jev
export CHAUFFEUR_JUDGE_ESCALATE=glide
export CHAUFFEUR_JUDGE_ESCALATE_BELOW=0.6        # the default
export CHAUFFEUR_JUDGE_REVIEW=system-one-review  # optional System Two step
```

- The primary (with its fallback, if any) answers every question first.
- Each answer under the bar, by effective confidence, is asked again of the
  escalation judge in one call with the same redacted state. Its answers
  replace those: it was benchmarked as the more accurate judge.
- With `CHAUFFEUR_JUDGE_REVIEW`, answers still under the bar go to that
  System Two profile, which answers each from the state or abstains. An
  abstention keeps the escalation judge's answer. System Two must be enabled
  in `judge.json`; an unknown profile stops startup. The review sends the most
  recent state that fits its 32 KiB input limit, and reviews at most 16
  questions per call.
- Every step is best effort. A failed, invalid, or late step keeps the
  answers already in hand; only a primary failure fails the judgment.
- Steps share the signal's deadline. The escalation call starts only with
  500 ms left and the review only with 2 s left, keeping 1 s for the
  engine's next round. Inside the 2-second permission budget, the review
  normally does not run.

A synthetic replay of 54 Chauffeur cases (two repeats) with the 0.6 bar
measured 95.8% accuracy against Jev's 90.5%, with 3 of 108 requests needing
System Two. The bar was chosen on that same set, not held-out data; confirm
it on recorded traffic before relying on it.

## Images and retention

SubQ supports image input, but Chauffeur's `SystemOne::ask` currently accepts
text and questions only. Selecting SubQ does not enable screenshot collection
or image uploads. That needs a separate input-path change and image privacy
handling.

SubQ's documented default retention is indefinite. An organization's policy
can be changed to a fixed period or zero retention. The Decisions API does not
document a per-request `store: false` switch. Enabling fallback therefore
explicitly authorizes sending the redacted judgment context to that provider;
it is never inferred from finding an API key.

## System Two

System Two answers judgments that need reasoning over evidence, with a
general-purpose LLM. It ships off. Each reasoning names a profile in
[`skills/config/judge.json`](../skills/config/judge.json), which picks:

- the provider plugin and its backend;
- the model, its reasoning effort, and output limit;
- a timeout for each call.

The shipped `completion-review` and `skill-review` profiles use GPT-6 Luna
through the `openai/codex` backend; `skill-review` reviews a skill before
`chauffeur skill approve` adds it (see [remote skills](remote-skills.md#the-safety-review)). That backend uses your ChatGPT subscription through
the Codex CLI's login:

- It reads `$CODEX_HOME/auth.json`, else `~/.codex/auth.json`. Log in with
  `codex login` first.
- It refreshes the access token shortly before it expires and writes the new
  tokens back, keeping the file's other fields and its `0600` mode, so the
  Codex CLI stays logged in.
- Its calls count against your ChatGPT plan's limits.
- It asks OpenAI not to store the request (`store: false`). The evidence it
  sends is redacted locally first, as System One's is.

To turn it on, add to `~/.config/chauffeur/judge.json`:

```json
{ "system_two": { "enabled": true } }
```

The same file can override any profile field or the wording, such as
`{ "system_two": { "profiles": { "completion-review": { "effort": "medium" } } } }`.
Edits apply without a restart. A file that fails to load keeps the previous
profiles running and logs why; at startup it stops the daemon, as other
config does.

A verdict must fit the request's schema and cite only evidence it was given.
A refusal, cut-off output, timeout, or an unknown profile gives no verdict,
and the capability applies its failure posture.

Each call shares its signal's remaining host budget. The daemon reserves
150 ms for the reply and caps the run at 14 seconds; the profile timeout is
an upper bound. System Two calls with less than 250 ms left return a timeout without
starting provider inference. The OpenCode adapter supplies its hook timeout,
including the 2-second permission budget. Callers omitting `deadline_ms` retain
the daemon's default limit.

`audit.jsonl` records each reasoning's profile, provider/backend, requested model,
reported model and input/output token usage when available, latency in milliseconds,
and verdict or failure. Text remains clipped and redacted. Missing usage is
omitted. A timed-out run retains settings for calls that started; a blocking
provider call can still occupy its worker until its own timeout, and its late
result is discarded.

### Opt-in rulebook completion review

The shipped goal's `goal-done` step has a disabled `review` block. Enabling
System Two alone leaves `/goal` unchanged. Opting in also requires setting
`review.enabled: true` in the loaded `rulebooks/goal.json` and starting the
book. See the [rule review format and outcome table](../skills/README.md#optional-system-two-review).

The route screens once with System One and reviews selected unsure/failed
answers once with the named profile. A confident yes completes without review;
a confident no cannot escalate. Only a cited `complete` verdict can turn an
unsure/failed completion screen into a holding step. Unavailable execution
preserves that screen's non-holding outcome. Todo gates and rule priorities
remain authoritative.

Sixteen synthetic labeled cases live in
[`bench/judges/completion-review.json`](../bench/judges/completion-review.json).
An ignored replay uses the real daemon profile registry, an isolated config
under `target/`, and the same rule evidence and core validation:

```sh
cargo test -p chauffeur-daemon completion_review::completion_review_live --locked -- --ignored --nocapture
```

It requires `TYPESAFE_API_KEY` and a Codex login; it may refresh and write that
login. It never contacts a running daemon or changes live Chauffeur config.
Each case reports verdict, completion agreement and latency for System One
alone, screen→review, and direct review, followed by p50/p95. The first two
variants share one classification call; direct review makes its own profile
call without classification I/O. All variants retain the todo veto. The corpus
isolates `goal-done`; it does not score pause/continuation selection. These
synthetic expectations need independent human review and broader trajectories
before default enablement.

The [single-pass evaluation report](../bench/judges/completion-review.md)
records agreement, latency and an evidence limitation: recent activity does not
retain successful tool-result output. A closing report can therefore remain
insufficient to establish completion even when the scenario includes passing checks.

## Replay benchmark

SubQ is included in `bench/judges.json`. Replay a redacted corpus recorded by
the daemon with both providers:

```sh
cargo run -q -p chauffeur-bench -- judges /path/to/judgments.jsonl \
  --only jev,subq --out /path/to/results
```

Agreement with recorded Jev answers is not ground-truth accuracy. Independently
label disputes before changing the default judge.

## Sources

- [Fastino GLiDE API and TypeSafe migration](https://docs.fastino.ai/inference/systemone.md)

- [SubQ Decisions API](https://docs.subq.ai/endpoints/decisions-api/)
- [SubQ models and pricing](https://docs.subq.ai/using-the-api/models/)
- [SubQ data retention](https://docs.subq.ai/using-the-api/data-retention/)
