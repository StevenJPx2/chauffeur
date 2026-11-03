# `judge`

The agent asks Chauffeur's judges a question about evidence it has, and gets
the answer with every outcome's probability. It is one step through
[`navigate`](../plans/pluggability.md#chauffeur-navigate), so confidence,
model fallback, redaction and keys stay in the engine, and the run appears in
[`chauffeur audit`](../cli/audit.md).

| Input        | Type                                                          | Default                          |
| ------------ | ------------------------------------------------------------- | -------------------------------- |
| `question`   | string                                                        | Required                         |
| `evidence`   | object of named text blocks                                   | Required; at most 256 KiB in all |
| `outcomes`   | 2 to 64 strings, in order when they are a scale               | `["yes", "no"]`                  |
| `confidence` | 0 to 1, how likely the top outcome must be                    | None: the likeliest outcome      |
| `tier`       | `"system-one"`, or `"system-two"` where the user turned it on | `"system-one"`                   |

| Reply                                  | When                                                              |
| -------------------------------------- | ----------------------------------------------------------------- |
| `{ "outcome", "p", "level", "model" }` | The judge answered; `outcome` may be `"unsure"`                   |
| `{ "failed": "<reason>" }`             | `unavailable`, `rate_limited`, `timeout`, `invalid` or `deadline` |

## Yes or no

```json
{
  "question": "Does this diff change the package's public API?",
  "evidence": { "diff": "--- a/src/lib.rs\n+++ b/src/lib.rs\n-pub fn parse(…" }
}
```

```json
{ "outcome": "yes", "p": { "yes": 0.91, "no": 0.09 }, "level": 0.09, "model": "jev" }
```

## A choice

```json
{
  "question": "Which team should review this change?",
  "outcomes": ["frontend", "backend", "infra"],
  "evidence": { "files": "terraform/main.tf\n.github/workflows/deploy.yml" }
}
```

```json
{
  "outcome": "infra",
  "p": { "frontend": 0.02, "backend": 0.07, "infra": 0.91 },
  "level": 1.89,
  "model": "jev"
}
```

## A bar it must clear

```json
{
  "question": "Is this log line an error the user needs to see?",
  "evidence": { "line": "WARN retrying request 2/3" },
  "confidence": 0.8
}
```

```json
{
  "outcome": "unsure",
  "p": { "yes": 0.62, "no": 0.38 },
  "level": 0.38,
  "model": "jev"
}
```

## From Code Mode

In OpenCode, MCP tools sit under their server's namespace inside `execute`:

```ts
const flaky = await tools.chauffeur.judge({
  question: "Does this test failure look flaky rather than caused by the change?",
  evidence: { failure: log, diff }
})
if (flaky.outcome === "yes") await rerun()
```

A failure, such as no daemon, comes back as `{ "failed": … }`, never as an
answer, so the agent can tell "no" from "could not ask".
