# `ask-chauffeur`

Registers the `ask_chauffeur` tool: the agent asks in plain words for a tool
or skill it lacks, and gets the best match back. Slice 6. How the agent calls
it, with example replies, is on the [tool's page](../tools/ask-chauffeur.md).

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
      run: (input, e) => grant(input.need, e) // tools and skills, in one round
    })
  }
})
```

`grant` runs one inline strategy:

| Step    | Asks                                                                                | Keeps                                   |
| ------- | ----------------------------------------------------------------------------------- | --------------------------------------- |
| Tools   | A yes/no per hidden group and Code Mode namespace: is this what the need calls for? | `select` `top: 2`, `min: 0.7`           |
| Skills  | A yes/no per skill among the 24 closest to the need, not yet given                  | `select` `min: 0.7`                     |
| Pick    | A `choice` among the skills that passed, run only when two or more did              | One skill                               |
| Closest | When nothing passed                                                                 | `select` `min: 0.3`, named in the reply |

The reply carries the revealed tools' best matches, and the chosen skill's
body with its base directory. The tool is in the [`tools`](tools.md)
plugin's `base`, so it is never hidden, and the adapter leaves its calls out
of `tool.after`, so no rule judges them.
