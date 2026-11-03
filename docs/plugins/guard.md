# `guard`

Stops irreversible harm before a command runs. Patterns catch the known
cases at once; System One judges the rest; a command it blocked once is
blocked by pattern from then on. Slice 5.

| Hook                                             | Does                                               |
| ------------------------------------------------ | -------------------------------------------------- |
| `ctx.before("tool.before")`, shell calls         | Patterns, then the harm question                   |
| `ctx.before("permission.asked")`, shell requests | The same, for requests the harness would ask about |

| Check                                                        | Then                                                                                                     |
| ------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------- |
| A `patterns` entry in `backstop.json`, shipped or the user's | Block, before any model call                                                                             |
| A `confirm` entry, such as a force-push to `main`            | Ask the user, before any model call                                                                      |
| A command on the learned list                                | Block                                                                                                    |
| "Could this command cause irreversible harm?"                | `yes` blocks and adds the command to the learned list; `unsure`, `failed` or a slow answer asks the user |

A pattern that names a root matches only the root, so `rm -rf /tmp/build`
passes. The task-approval and shell-rewrite checks are rule files, not this
plugin: see [Permissions](../migration/permission.md).

```ts
import { Plugin } from "@fdcn/chauffeur/plugin"
import { ask, judge, noul } from "@fdcn/chauffeur/judge"

const harm = judge({
  tier: "system-one",
  model: "jev",
  confidence: { yes: 0.9, no: 0.95 }
})

export default Plugin.define({
  id: "guard",
  setup(ctx) {
    // Patterns, confirm, learned.
    const backstop = loadBackstop(ctx.options, ctx.storage)
    ctx.before(
      "tool.before",
      async (e) => {
        if (e.tool !== "shell") return
        const matched = backstop.match(e.input.command)
        if (matched?.kind === "block") return e.block(matched.text)
        if (matched?.kind === "confirm") return e.askUser(matched.text)
        const harmful = await ask(
          noul("Could this command cause irreversible harm?"),
          {
            judge: harm,
            evidence: { command: e.input.command }
          }
        )
        if (harmful.ok && harmful.outcome === "yes") {
          backstop.learn(e.input.command)
          e.block("This command may cause irreversible harm.")
        } else if (!harmful.ok || harmful.outcome === "unsure")
          e.askUser("Chauffeur isn't sure this is safe.")
      },
      { budget: 2_000, onTimeout: "ask" }
    )
  }
})
```

## Commands

```sh
# What the guard would do, without running it.
chauffeur guard check 'git push --force origin main'
# The commands it learned to block.
chauffeur guard learned
# Take one off the learned list.
chauffeur guard forget 'rm -rf ./build'
```

```text
$ chauffeur guard check 'git push --force origin main'
ask: matches confirm pattern "push --force origin main"
```

`check` asks the harm question only when no pattern decides, and says which
model answered.

## Options

```json
{ "guard": { "options": { "backstop": "~/.config/chauffeur/backstop.json" } } }
```

The user's `backstop.json` adds patterns, or replaces the shipped ones with
`"replace": true`. The learned list is `learned.json` in the plugin's
storage, apart from configuration, so it can be reviewed and pruned.
