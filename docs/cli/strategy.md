# `chauffeur strategy show`

Arrives with slice 3. Draws a rule's strategy as a tree: its steps, what each
waits for, the models each resolves to and their confidence, and the most
rounds and model calls it can take.

```sh
chauffeur strategy show sourcefed/triage
chauffeur strategy show --file ./rules/draft.json
```

The playground's Strategy tab steps through the same tree.
