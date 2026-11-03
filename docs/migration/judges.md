# Judges

v1 chose its judge with environment variables and routed some questions to
code with `judge.json`. In v2 every model is an entry in `models.json`, each
tier lists its models in fallback order, and a rule names the judge for each
step.

| v1                                                         | v2                                                                                  | Slice          |
| ---------------------------------------------------------- | ----------------------------------------------------------------------------------- | -------------- |
| `CHAUFFEUR_JUDGE=jev` or `subq`                            | The first model in `tiers["system-one"]`                                            | 1              |
| `CHAUFFEUR_JUDGE_FALLBACK`                                 | A second model in the same list                                                     | 1, `subq` in 3 |
| A 3-second timeout per attempt, no retry loop              | `timeout_ms` per model, `3000` for Jev                                              | 1              |
| After a failure the primary rests for 60 s, then is probed | The tier skips a model for 60 s after it fails, then tries it again                 | 3              |
| Failure or an invalid answer falls back                    | Unavailable, rate-limited or timed out falls back; an invalid answer fails the step | 1              |
| `judge.json` `routes` to the facts judge                   | A step with `"judge": "facts"`                                                      | 4              |
| `judge.json` `system_two.profiles`                         | `models.json` `system-two` entries, `openai` and `anthropic` formats                | 11             |
| `situation.json` texts and clipping                        | Evidence blocks, rendered by the adapter; see [Rule files](rule-format.md#evidence) | 1, 5           |

```jsonc
// ~/.config/chauffeur/models.json: Jev first, SubQ when Jev cannot answer
{ "tiers": { "system-one": ["jev", "subq"] } }
```

The format details, confidence and placeholders are in the plan's
[Judges](../plans/pluggability.md#judges).
