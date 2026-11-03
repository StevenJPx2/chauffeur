# Slice 11: Review by a reasoning model

**Status:** Planned.

**You can:** escalate to a reasoning model, such as the Slack review in
[Event triage](../pluggability.md#event-triage) and the skills plugin's review step.

This slice adds the `openai` and `anthropic` formats, in
`crates/judges/src/tiers/system_two/`. A System Two answer is
schema-checked against the question's outcomes, and its `cites` must name
evidence the step was given. Model entries set the model ID, effort, timeout
and output budget. System Two ships disabled until a model and its
evaluations are reviewed.

Verification: loopback cases for valid, refused, malformed, truncated,
unavailable and timed-out replies; citations of evidence the step lacks are
rejected; the 32 KiB input budget; a slow reasoning call cannot consume
reserved interactive System One capacity; human-labelled cases replay before
a live model is enabled.
