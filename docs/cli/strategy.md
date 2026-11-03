# `chauffeur strategy`

Arrives with slice 3. Shows the strategies of the loaded rules: what each asks,
of which judges, in what order, and how many rounds and model calls it can
take at most.

## `chauffeur strategy list`

One line per loaded rule: its ID, the event it hooks, its steps, and its most
rounds and model calls.

```sh
chauffeur strategy list
```

```text
conventional-commits     tool.after               1 step    1 round   1 model call
sourcefed/triage         sourcefed.event          3 steps   2 rounds  2 model calls
opencode-history/recall  opencode-history.recall  2 steps   2 rounds  1 model call
```

## `chauffeur strategy show`

Draws one rule's strategy as a tree: its steps, what each waits for, the
models each resolves to and their confidence.

```sh
chauffeur strategy show sourcefed/triage
chauffeur strategy show --file ./rules/draft.json   # a rule that is not loaded yet
```

The playground's Strategy tab steps through the same tree.
