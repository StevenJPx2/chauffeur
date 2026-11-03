# `warming`

Keeps an idle session's prompt cache warm, so the user's next prompt reads it
instead of re-sending the conversation, for as long as that is cheaper.
Slice 7.

| Hook                           | Does                                                       |
| ------------------------------ | ---------------------------------------------------------- |
| `ctx.on("session.idle")`       | Schedules keep-alives every `cache_seconds − lead_seconds` |
| Any later event in the session | Stops its schedule                                         |

```ts
import { Plugin } from "@fdcn/chauffeur/plugin"
import { session } from "@fdcn/chauffeur/harness"

export default Plugin.define({
  id: "warming",
  setup(ctx) {
    ctx.on("session.idle", (e) =>
      schedule(e, ctx.options, () => session.warm(e.session))
    )
  }
})
```

- **How many:** as many as fit within `max_seconds` of the turn's end and
  cost less than one re-send: a keep-alive reads the context at the
  cache-read price, a cold prompt writes it at the cache-write price. That is
  14 on Claude Opus 5.5.
- **Never warmed:** a context under `min_tokens`, an unknown cache-read
  price, a provider that hit a limit, or a subagent's session.
- **In OpenCode,** the harness package carries `session.warm` out as one
  transient generate request with the keep-alive prompt, as v1 did. Where a
  harness can neither, the plugin does nothing. Leave the harness's own
  warming setting off, or the session is warmed twice.

## Options

```json
{
  "warming": {
    "options": { "max_seconds": 3600, "lead_seconds": 60, "min_tokens": 20000 }
  }
}
```
