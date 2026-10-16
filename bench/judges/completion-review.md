# Completion review evaluation

`completion-review.json` contains 16 synthetic scenarios with expected verdicts.
Labels are test expectations, not independently reviewed ground truth.

The daemon's ignored `completion_review::completion_review_live` test runs the
shipped `completion-review` profile (`openai/codex`, `gpt-6-luna`, low effort,
6-second timeout) and Jev with the shipped completion thresholds. It uses an
isolated config and never enables a running rulebook.

```sh
cargo test -p chauffeur-daemon completion_review::completion_review_live --locked -- --ignored --nocapture
```

## Single-pass results

Agreement scores the completion decision: only the `complete` label expects a
holding completion step. There are four positive and twelve negative cases.
The open-todo case is vetoed before inference in every variant. Latency includes
inference and judgment continuation, excludes fixture setup, and includes the
zero-latency veto case. The System One call is shared between the first two variants.

| Variant | Completion agreement | p50 / p95 |
|---|---|---|
| System One | 13/16 | 275 / 406 ms |
| Screen → review | 14/16 | 281 / 3124 ms |
| Direct review | 13/16 | 1825 / 3458 ms |

| Variant | False completions | Missed completions |
|---|---|---|
| System One | 0/12 | 3/4 |
| Screen → review | 0/12 | 2/4 |
| Direct review | 0/12 | 3/4 |

Screened review escalates four cases and completes the finished-haiku case that
System One leaves unsure. Both review variants reject `checked-fix` and
`checked-multi-part` as insufficient evidence. Direct review also rejects
`measured-target`, which System One accepts. All calls return valid outcomes;
the run takes 44.88 seconds, with 15 classification calls and 19 reasoning calls.

## Evidence boundary and rollout

The fixture's `activity` text enters a `ToolResult.evidence` field. Core's
recent-activity rendering retains tool names and inputs, plus failures, but
does not retain successful tool-result output. The reviews therefore see the
closing report and recorded verification call, without the fixture's successful
command output. The labels express the scenario's desired completion; this
replay exposes the gap between those facts and the evidence available at turn end.

The reported agreement does not establish production quality. Richer bounded
verification evidence, independently reviewed labels, broader trajectories and
deadline measurements remain rollout requirements. The shipped review stays off.
