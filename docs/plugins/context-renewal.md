# `context-renewal`

When the user comes back to a long conversation after its prompt cache has
expired, has a cheaper model summarize it before the prompt runs. The prompt
still runs on the session's own model. Slice 7.

| Hook                        | Does                                                                                                |
| --------------------------- | --------------------------------------------------------------------------------------------------- |
| `ctx.on("session.resumed")` | Renews when the session was idle at least its cache lifetime and its context is at least the budget |
| `compaction.before`         | Writes the summary in a temporary child session on the summary model, with the harness's template   |

No question is asked: the decision is two comparisons.

```ts
import { Plugin } from "@fdcn/chauffeur/plugin"
import { session } from "@fdcn/chauffeur/harness"

export default Plugin.define({
  id: "context-renewal",
  setup(ctx) {
    ctx.on("session.resumed", (e) => {
      const cached =
        ctx.options.cache_seconds[e.provider] ?? ctx.options.cache_seconds["*"]
      if (
        e.idle_seconds >= cached &&
        e.tokens.used >= budget(e.tokens.window, ctx.options.budget)
      )
        session.compact(e.session, { model: summaryModel(e, ctx) })
    })
  }
})
```

- **The budget** is a share of the window, kept between a floor and a
  ceiling: a 1M-token window renews at 100k, a 200k one at 50k.
- **The summary model** is a `pins` entry, else a fast-tier model on the
  agent's own provider, then on another provider used directly, then a free
  one, from the [model router's](model-router.md) provider tables; never a
  blacklisted model or a provider that hit a limit.
- **Left alone:** a session already on a fast-tier or free model, which the
  harness compacts as usual.
- **The summary** is written with `session.create`, `session.generate` and
  `session.remove`, so it carries the provider's sign-in. A summary that
  fails passes over its provider for an hour, kept in the plugin's storage.

## Commands

```sh
# The summary model each provider would use, and providers passed over.
chauffeur context-renewal status
```

```text
$ chauffeur context-renewal status
PROVIDER    SUMMARY MODEL                  PASSED OVER
anthropic   anthropic/claude-haiku-5-5     -
openai      openai/gpt-6-luna              until 14:52: usage limit
```

## Options

```json
{
  "context-renewal": {
    "options": {
      "cache_seconds": {
        "anthropic": 300,
        "openai": 1800,
        "deepseek": 3600,
        "*": 300
      },
      "budget": { "share": 0.215, "min_tokens": 50000, "max_tokens": 100000 },
      "pins": []
    }
  }
}
```
