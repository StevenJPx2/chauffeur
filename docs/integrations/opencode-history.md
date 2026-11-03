# opencode-history

opencode-history indexes past OpenCode sessions. When the user sends a
message it searches the index, asks a System One judge whether each passage
would help the agent answer it, and attaches the best.

With Chauffeur, the ranking is the `opencode-history/recall` rule in the
plan's [per-turn recall](../plans/pluggability.md#per-turn-recall).
opencode-history keeps its search and the attaching; the judge, the bar and
the top-three cut are the rule's.

## The call

```ts
import { navigate } from "@fdcn/chauffeur/client"

const { intents } = await navigate(
  {
    on: "opencode-history.recall",
    evidence: { message },
    candidates: Object.fromEntries(passages.map((p) => [p.id, p.text]))
  },
  { signal: AbortSignal.timeout(1_500) }
)
const attach = intents.flatMap((intent) => intent.attach ?? [])
```

An error or a missed deadline attaches nothing, as a failed Jev call does.

## What it needs

| Piece                                             | Slice |
| ------------------------------------------------- | ----- |
| `navigate` and the TypeScript client              | 2     |
| `for_each` over candidates, and the `select` fact | 4     |
