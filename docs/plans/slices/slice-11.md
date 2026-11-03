# Slice 11: Review by a reasoning model

**Status:** Planned.

**You can:** escalate to a reasoning model, such as the Slack review in
[Event triage](../pluggability.md#event-triage) and the skills plugin's review step.

## What it builds

### The System Two formats

`openai` and `anthropic`. A System Two answer is schema-checked against the
question's outcomes, and its `cites` must name evidence the step was given.
Model entries set the model ID, effort, timeout and output budget. System Two
ships disabled until a model and its evaluations are reviewed.

Files: `crates/judges/src/tiers/system_two/`

Verified by:

- Loopback cases for valid, refused, malformed, truncated, unavailable and
  timed-out replies.
- Citations of evidence the step lacks are rejected.
- The 32 KiB input budget.
- A slow reasoning call cannot consume reserved interactive System One
  capacity.

## End to end

- Human-labelled cases replay before a live model is enabled.
