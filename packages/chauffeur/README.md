# `@fdcn/chauffeur`

Asks Chauffeur from any TypeScript program, over the daemon's socket.

| Entry     | Holds                                                                           |
| --------- | ------------------------------------------------------------------------------- |
| `/client` | `navigate` on a connection of its own, `connect` for many, and the wire's types |
| `/judge`  | `judge`, `noul` and `ask`: one question, its result typed from its outcomes     |

| File                       | Holds                                                                  |
| -------------------------- | ---------------------------------------------------------------------- |
| `src/client/wire.ts`       | Requests, intents, verdicts, events and `NavigateError`, as the schema |
| `src/client/socket.ts`     | `socketPath`: the option, `$CHAUFFEUR_SOCKET`, then the runtime folder |
| `src/client/connection.ts` | `Connection`: calls by ID on one socket, events routed, cancelling     |
| `src/client/navigate.ts`   | `navigate` and `connect`                                               |
| `src/judge/index.ts`       | `judge`, `noul`, `ask`                                                 |

```ts
import { navigate } from "@fdcn/chauffeur/client"
import { ask, judge, noul } from "@fdcn/chauffeur/judge"

const { intents } = await navigate(
  { on: "sourcefed.event", evidence: { event, thread } },
  { signal: AbortSignal.timeout(3_000) }
)

const quick = judge({ tier: "system-one", confidence: 0.7 })
const done = await ask(noul("Is the migration finished?"), {
  judge: quick,
  evidence: { log }
})

if (done.ok && done.outcome === "yes") {
  /* "yes" | "no" | "unsure" */
}
```

- `navigate` rejects with a `NavigateError` whose `reason` is the daemon's,
  or `unavailable` when no daemon answers, it speaks another protocol, or it
  has not answered two seconds past the deadline. An aborted signal rejects
  with the signal's reason, and the daemon cancels the call.
- `ask` never rejects; anything but an answer is `{ ok: false, reason }`.
- `connect()` keeps one connection, opened again on the next call after the
  daemon restarts; `close()` ends it.

```sh
bun install
bun run check   # types, tests and the build
```

The tests against the real daemon run when `CHAUFFEUR_BIN` names a built
`chauffeur`, as CI does; they start and stop it, so they stay off a
developer's machine.
