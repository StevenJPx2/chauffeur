# Hosted decision judges

Chauffeur supports two System One providers:

- **Jev**, from TypeSafe, the default (`jev-latest`).
- **SubQ Snap**, optional (`subq-snap-preview`).

Both receive the same locally redacted state and typed questions. They share
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

Provider-specific overrides:

- `TYPESAFE_BASE_URL`, default `https://api.typesafe.ai`.
- `TYPESAFE_DEFAULT_MODEL`, default `jev-latest`.
- `SUBQ_BASE_URL`, default `https://api.subq.ai`.
- `SUBQ_DEFAULT_MODEL`, default `subq-snap-preview`.

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

The shipped `completion-review` profile uses GPT-6 Luna through the
`openai/codex` backend. That backend uses your ChatGPT subscription through
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

- [SubQ Decisions API](https://docs.subq.ai/endpoints/decisions-api/)
- [SubQ models and pricing](https://docs.subq.ai/using-the-api/models/)
- [SubQ data retention](https://docs.subq.ai/using-the-api/data-retention/)
