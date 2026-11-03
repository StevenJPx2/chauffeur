# Slice 7: Context renewal, warming and model failover

**Status:** Planned.

**You can:** have a long, cold session compacted with a cheap model before
your next prompt runs, an idle session's prompt cache kept warm, and the
agent moved to another provider's model when it hits a usage limit, then
back once the limit clears, as in
[Context renewal and warming](../../migration/renewal-and-warming.md) and
[Model router](../../migration/model-router.md).

## What it builds

### The `context-renewal` and `warming` plugins

A long, cold session summarized by a cheaper model before the next prompt,
and an idle session's cache kept warm while that is cheaper, using the
session events and the `session.compact`, `session.warm` and `session.update`
actions the fidelity table marks as supported.

Files: `plugins/context-renewal/`, `plugins/warming/`

Commands: `chauffeur context-renewal status` and `chauffeur warming status`.

```ts
ctx.on("session.resumed", (e) => {
  if (
    e.idle_seconds >= cacheSeconds(e.provider) &&
    e.tokens.used >= budget(e.tokens.window)
  )
    session.compact(e.session, { model: summaryModel(e) })
})

ctx.on("session.idle", (e) => schedule(e, () => session.warm(e.session)))
```

Verified by:

- `main`'s `renewal.test.ts` and `warming.test.ts`.

### The `model-router` plugin

Failover on a usage limit and switch-back, with each provider's tier table.

Files: `plugins/model-router/`, `distribution/providers/`

Commands: `chauffeur model-router status` and `forget`.

```ts
ctx.on("response.finished", async (e) => {
  if (e.reason !== "error" || !isLimit(e.error)) return
  const pick = await ask(
    choice(candidates(e), "Which model should the agent continue on?"),
    { judge: quick, evidence: { error: e.error.message } }
  )
  session.update(e.session, {
    model: pick.ok && pick.outcome !== "unsure" ? pick.outcome : candidates(e)[0]
  })
  message.append(e.session, [{ text: "Switched model after a usage limit." }], {
    delivery: "wake"
  })
})
```

Verified by:

- `main`'s `routing.rs` and model-router tests.

## End to end

- In a real OpenCode, a session resumed after the cache expires is compacted
  before the prompt runs.
