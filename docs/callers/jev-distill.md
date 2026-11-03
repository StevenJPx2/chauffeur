# jev-distill

jev-distill trains a small student for one decision: a single JSON file that
predicts with no network, no key and no cost, in under a millisecond. Its
`serve` command answers `POST /v1/predict/<task>` on loopback.

To Chauffeur, a student is a System One model. A rule asks the student first
and asks Jev only what the student leaves unsure:

```jsonc
// ~/.config/chauffeur/models.json
{
  "models": {
    "routing-student": { "format": "student", "endpoint": "http://127.0.0.1:8787", "task": "routing" }
  }
}
```

```jsonc
// a rule's judges and strategy
"judges": {
  "local": { "tier": "system-one", "model": "routing-student", "confidence": 0.8 },
  "jev":   { "tier": "system-one", "model": "jev", "confidence": 0.7 }
},
"strategy": [
  { "id": "fast", "judge": "local", "ask": "route" },
  { "id": "sure", "judge": "jev", "ask": "route",
    "after": [{ "step": "fast", "when": ["unsure", "failed"] }] }
]
```

## The `student` format

| Chauffeur | Student |
|---|---|
| The question's outcomes | The task's labels; a question whose outcomes differ fails as `invalid` without a call |
| Evidence blocks | `state` fields, by name; a block the task does not declare is not sent |
| Probabilities | `probabilities`, one per label |
| Abstaining | Ignored: the rule's `confidence` decides `unsure` |
| An `experimental` task | Marked in the run's events, as a placeholder answer is |

Which labels a student may be trained on is jev-distill's concern; Chauffeur
only asks a student that exists.

## What it needs

| Piece | Slice |
|---|---|
| `after`, so Jev answers only what the student leaves unsure | 3 |
| The `student` format | 3 |
