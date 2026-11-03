# Seeing what it decided

Each command has its own page under [CLI](../cli/README.md).

| v1                                                      | v2                                                                                                                                                                          | Slice |
| ------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----- |
| `chauffeur audit [N]`, `--follow`, `--brief`            | The same command, over the same kind of file: one JSON line per run in `~/.local/state/chauffeur/audit.jsonl`, moved aside once past 8 MiB, readable with no daemon running | 1     |
| The audit log's `judges` field: who answered each round | Each step's answer names its model and whether it was a placeholder, with its probabilities and level                                                                       | 1     |
| `chauffeur skill validate PATH`                         | A rule that does not load is refused with its path and reason, listed by `chauffeur daemon status`                                                                          | 1     |
| `chauffeur skill validate-project WORKTREE`             | `chauffeur daemon status --project WORKTREE`: the rules a session there would load, and the refused ones                                                                    | 5     |
| `chauffeur skills`: sources and their freshness         | `chauffeur skills list`, a command the `skills` plugin adds: the harness's skills it sees, where each comes from, and whether a third-party one passed its screen           | 6     |
