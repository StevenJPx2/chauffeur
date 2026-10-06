import { Duration, Effect, Schedule } from "effect"
import { Host } from "./host.js"

/** The longest a first message waits for MCP servers to connect. */
const SETTLE: Duration.Input = "10 seconds"

const POLL: Duration.Input = "250 millis"

/**
 * Wait until no MCP server is still connecting, for at most ten seconds.
 * Right after the service starts, a context's first message can arrive while
 * servers connect; OpenCode builds that request's Code Mode catalog once they
 * have, so Chauffeur waits too and judges the namespaces the request will
 * carry. Any failure ends the wait: judging fewer namespaces is safe.
 */
export const mcpSettled: Effect.Effect<void, never, Host> = Effect.gen(function* () {
  const host = yield* Host

  const pending = Effect.suspend(() => host.mcp.list()).pipe(
    Effect.map((listed) => listed.data.some((server) => server.status.status === "pending")),
  )

  yield* pending.pipe(
    Effect.repeat({ schedule: Schedule.spaced(POLL), while: (stillPending) => stillPending }),
    Effect.timeout(SETTLE),
    Effect.catchCause(() => Effect.void),
  )
})
