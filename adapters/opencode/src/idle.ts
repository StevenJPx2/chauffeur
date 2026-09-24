import { Effect, Stream } from "effect"
import { Daemon } from "./daemon.js"
import { Host, type SessionID } from "./host.js"
import { signal, TEXT_CODE_POINTS } from "./protocol.js"
import { deliverContext } from "./skills.js"
import { clip, isIntegrationMessage } from "./text.js"

/**
 * Reports finished turns and delivers any context the engine returns, which
 * either wakes the idle agent or waits for its next turn.
 */
export const installIdle = Effect.gen(function* () {
  const host = yield* Host

  yield* host.event.subscribe().pipe(
    // A finished turn; an interrupted one means the user stopped the agent.
    Stream.runForEach((event) =>
      event.type === "session.execution.succeeded" || event.type === "session.execution.failed"
        ? reportTurnEnd(event.data.sessionID)
        : Effect.void),
    Effect.catch((error) => Effect.logError("chauffeur: idle events ended", error)),
    Effect.forkScoped,
  )
})

function reportTurnEnd(sessionID: SessionID): Effect.Effect<void, never, Host | Daemon> {
  return Effect.gen(function* () {
    const host = yield* Host
    const daemon = yield* Daemon

    // Unreadable session state still reports the turn, with empty fields.
    const [session, context] = yield* Effect.all([
      host.session.get({ sessionID }).pipe(Effect.orElseSucceed(() => undefined)),
      host.session.context({ sessionID }).pipe(Effect.orElseSucceed(() => [])),
    ], { concurrency: "unbounded" })

    const user = context.findLast((message) => message.type === "user" && !isIntegrationMessage(message.metadata))

    const effects = yield* daemon.signal(signal(String(sessionID), {
      type: "turn_end",
      workspace: session ? clip(String(session.location.directory), TEXT_CODE_POINTS) : "",
      user_request: user?.type === "user" ? clip(user.text, TEXT_CODE_POINTS) : "",
    }))

    const deliveries = effects.flatMap((effect) =>
      effect.type === "context" && effect.agent_id === String(sessionID) && (effect.delivery === "resume" || effect.delivery === "wait")
        ? [effect]
        : [])

    yield* Effect.forEach(deliveries, (effect) => deliverContext(sessionID, effect), { discard: true })
  }).pipe(
    // Fails open: nothing is delivered.
    Effect.catch((error) => Effect.logError("chauffeur: turn end not reported", error)),
  )
}
