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
