# Slice 7: Context renewal, warming and model failover

**Status:** Planned.

**You can:** have a long, cold session compacted with a cheap model before
your next prompt runs, an idle session's prompt cache kept warm, and the
agent moved to another provider's model when it hits a usage limit, then
back once the limit clears, as in
[Context renewal and warming](../../migration/renewal-and-warming.md) and
[Model router](../../migration/model-router.md).

| File                                               | Contents                                                                                                                                               |
| -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `plugins/{context-renewal,warming}/`               | The two plugins, using the session events and the `session.compact`, `session.warm` and `session.update` actions the fidelity table marks as supported |
| `plugins/model-router/`, `distribution/providers/` | Failover and switch-back, and each provider's tier table                                                                                               |

Verification: `main`'s `routing.rs`, `renewal.test.ts`, `warming.test.ts`
and model-router tests pass; in a real OpenCode, a session resumed after the cache expires is
compacted before the prompt runs.
