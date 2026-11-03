# `ask_chauffeur`

With `ask_chauffeur`, the agent asks in plain words for a tool or skill it
lacks. In v2 it is the `ask-chauffeur` plugin, in slice 6.

```ts
import { Plugin } from "@fdcn/chauffeur/plugin"
import { tool } from "@fdcn/chauffeur/harness"

export default Plugin.define({
  id: "ask-chauffeur",
  setup(ctx) {
    tool.add({
      name: "ask_chauffeur",
      description: "Ask for a tool or skill you lack, in plain words.",
      input: { need: "string" },
      run: (input, e) => grant(input.need, e), // tools and skills, in one round
    })
  },
})
```

| v1 | v2 |
|---|---|
| One `agent_request` signal; tool and skill exposure fan out in one Jev call | One inline strategy: a yes/no per hidden group and namespace, and per skill among the 24 closest to the need |
| A grant needs P ≥ 0.7 from Jev alone, with no escalation | `confidence: 0.7`, and no `after` step |
| At most two groups or namespaces and one skill; one more `choice` when several skills pass | `select` with `top: 2` for tools; `top: 1` for skills, after a `choice` step that runs only when two or more pass |
| Nothing granted: the closest candidates at P ≥ 0.3, suggested | A second `select` with `min: 0.3`, shown in the reply |
| The call is not reported as a tool result | The adapter leaves `ask_chauffeur` out of `tool.after` |
| Never hidden, never in Code Mode | `base` in the `tools` plugin's options |
