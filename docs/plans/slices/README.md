# Slices

A slice delivers one feature a person uses, through the real path from the
harness or caller to the judge and back. It builds only what that feature
needs, with the tests and proofs for that code. Every slice ends the same way:
the [quality gates](../pluggability.md#quality-gates) pass, its playground tab works in a real
browser, the feature works in its real harness or caller, the parity table
covers every `main` commit to date, an rc ships, a recording of the rc
working end to end goes on the slice's page, and work stops for review.

| #   | Slice                                                     | Status      |
| --- | --------------------------------------------------------- | ----------- |
| 0   | [Setup](slice-0.md)                                       | Implemented |
| 1   | [Conventional Commits nudge in OpenCode](slice-1.md)      | Implemented |
| 2   | [sourcefed triage](slice-2.md)                            | In progress |
| 3   | [Escalation from Jev to GLiDE](slice-3.md)                | Planned     |
| 4   | [opencode-history recall](slice-4.md)                     | Planned     |
| 5   | [Harm guard, permissions and v1's rules](slice-5.md)      | Planned     |
| 6   | [Skills and tools](slice-6.md)                            | Planned     |
| 7   | [Context renewal, warming and model failover](slice-7.md) | Planned     |
| 8   | [Pi](slice-8.md)                                          | Planned     |
| 9   | [Claude Code](slice-9.md)                                 | Planned     |
| 10  | [Codex](slice-10.md)                                      | Planned     |
| 11  | [Review by a reasoning model](slice-11.md)                | Planned     |
| 12  | [Rulebooks](slice-12.md)                                  | Held        |
| 13  | [Cutover](slice-13.md)                                    | Planned     |

The work goes one slice at a time, with review before the next starts.
Cutover, slice 13, waits for rulebooks, since v1 users rely on `/goal`.
