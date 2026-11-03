# Seeing what it decided

| v1                                                      | v2                                                                                                                                                 | Slice                          |
| ------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------ |
| `chauffeur audit --follow --brief`                      | `chauffeur events --follow`                                                                                                                        | `events` in 1, `--follow` in 5 |
| The audit log's `judges` field: who answered each round | Each step's answer names its model and whether it was a placeholder                                                                                | 1                              |
| `chauffeur skill validate PATH`                         | A rule that does not load is refused with its path and reason, listed by `chauffeur status`                                                        | 1                              |
| `chauffeur skill validate-project WORKTREE`             | `chauffeur status --project WORKTREE`: the rules a session there would load, and the refused ones                                                  | 5                              |
| `chauffeur skills`: sources and their freshness         | `chauffeur skills`, from the `skills` plugin: the harness's skills it sees, where each comes from, and whether a third-party one passed its screen | 6                              |
