import { Duration, Effect, Fiber, type Scope, Stream } from "effect"
import { Daemon } from "./daemon.js"
import { Host, type SessionID } from "./host.js"
import { ref } from "./model-router.js"
import { type CachePrices, type HostEffect, type Signal, signal } from "./protocol.js"
import { contextFacts } from "./renewal.js"
import { Texts } from "./texts.js"

const MAX_SESSIONS = 256

/** How long one warming request may take before it counts as failed. */
const WARM_TIMEOUT: Duration.Input = "2 minutes"

/** One price tier of a model the host lists, in dollars per million tokens. */
type Cost = {
  readonly tier?: { readonly size: number } | undefined
  readonly input: number
  readonly cache: { readonly read: number; readonly write: number }
}

export type WarmingControl = {
  /** When a warming request last refreshed the session's prompt cache, in epoch milliseconds. */
  readonly lastWarmed: (sessionID: string) => number | undefined
}

/**
 * Keeps an idle session's prompt cache warm. When a session the user drives
 * finishes its turn, the engine decides from the model's cache prices how
 * often to warm it and how many times; this sends each warming request,
 * OpenCode's transient `session.generate` on the session's context, and
 * stops at the session's next request of its own. Failed requests are logged
 * and the next one is still sent.
 */
export const installWarming: Effect.Effect<WarmingControl, never, Host | Daemon | Texts | Scope.Scope> = Effect.gen(function* () {
  const host = yield* Host
  const daemon = yield* Daemon
  const texts = yield* Texts
  const scope = yield* Effect.scope
  const loops = new Map<string, Fiber.Fiber<void>>()
  const warmed = new Map<string, number>()
  /** Executions started per session, so a turn that ends after the next began is not warmed. */
  const executions = new Map<string, number>()

  const started = (sessionID: string) => {
    if (!executions.has(sessionID) && executions.size >= MAX_SESSIONS) executions.clear()

    executions.set(sessionID, (executions.get(sessionID) ?? 0) + 1)

    return stop(sessionID)
  }

  const stop = (sessionID: string) => {
    const loop = loops.get(sessionID)

    loops.delete(sessionID)

    // Interrupted in the background, so the session's own request never waits on it.
    return loop ? Fiber.interrupt(loop).pipe(Effect.forkIn(scope), Effect.asVoid) : Effect.void
  }

  const warm = (sessionID: SessionID, everySeconds: number, times: number) => Effect.gen(function* () {
    for (let sent = 0; sent < times; sent += 1) {
      yield* Effect.sleep(Duration.seconds(everySeconds))
      yield* host.session.generate({ sessionID, prompt: texts.current().warming.prompt }).pipe(
        Effect.timeout(WARM_TIMEOUT),
        Effect.andThen(() => Effect.sync(() => {
          if (!warmed.has(String(sessionID)) && warmed.size >= MAX_SESSIONS) warmed.clear()

          warmed.set(String(sessionID), Date.now())
        })),
        Effect.catch((error) => Effect.logWarning("chauffeur: a warming request failed", error)),
      )
    }
  })

  const schedule = (sessionID: SessionID) => Effect.gen(function* () {
    const id = String(sessionID)
    const execution = executions.get(id)
    const idle = yield* idleSignal(sessionID)

    if (!idle) return

    const plan = warmPlan(yield* daemon.signal(idle), id)

    yield* stop(id)

    // A turn that began while the engine decided keeps its own cache warm.
    if (!plan || executions.get(id) !== execution) return

    if (loops.size >= MAX_SESSIONS) yield* Effect.forEach([...loops.keys()], stop, { discard: true })

    loops.set(id, yield* warm(sessionID, plan.every, plan.times).pipe(Effect.forkIn(scope)))
  }).pipe(
    // Warming fails open: the cache expires as it would without Chauffeur.
    Effect.catch((error) => Effect.logError("chauffeur: warming unavailable", error)),
  )

  // Any request of the session's own ends its warming; a warming request is a `generate`, not a `context`.
  yield* host.session.hook("context", (event) => stop(String(event.sessionID)))

  yield* host.event.subscribe().pipe(
    Stream.runForEach((event) => {
      if (event.type === "session.execution.started") return started(event.data.sessionID)

      return event.type === "session.execution.succeeded"
        ? schedule(event.data.sessionID).pipe(Effect.forkIn(scope), Effect.asVoid)
        : Effect.void
    }),
    Effect.catch((error) => Effect.logError("chauffeur: warming events ended", error)),
    Effect.forkScoped,
  )

  return { lastWarmed: (sessionID) => warmed.get(sessionID) }
})

/**
 * What a finished turn left in the cache, for the engine: the session's
 * model, context size and prices. Undefined for a subagent's session, one
 * in another location, or a context without a measured reply.
 */
function idleSignal(sessionID: SessionID): Effect.Effect<Signal | undefined, unknown, Host> {
  return Effect.gen(function* () {
    const host = yield* Host
    const session = yield* host.session.get({ sessionID })

    // Only the session's own location reports it.
    if (session.parentID !== undefined || String(session.location.directory) !== String(host.location.directory)) return undefined

    const facts = contextFacts(yield* host.session.context({ sessionID }), Date.now())

    if (!facts) return undefined

    const model = session.model ?? facts.model
    const listed = (yield* host.model.list()).data.find((entry) => entry.providerID === model.providerID && entry.id === model.id)

    return signal(String(sessionID), {
      type: "context_idle",
      model: ref(model),
      context_tokens: facts.tokens,
      prices: pricesAt(listed?.cost ?? [], facts.tokens),
    })
  })
}

/** The engine's warming for session `id`, if it asked for any. */
function warmPlan(effects: ReadonlyArray<HostEffect>, id: string): { readonly every: number; readonly times: number } | undefined {
  const plan = effects.find((effect) => effect.type === "warm" && effect.agent_id === id)

  return plan?.type === "warm" && plan.times > 0 && plan.every_seconds > 0 ? { every: plan.every_seconds, times: plan.times } : undefined
}

/**
 * The model's prices for a `tokens`-token prompt: the tier for the largest
 * context size the prompt reaches, in millionths of a dollar per million
 * tokens. All 0 for a model the host lists no prices for.
 */
export function pricesAt(cost: ReadonlyArray<Cost>, tokens: number): CachePrices {
  const tier = cost
    .filter((entry) => (entry.tier?.size ?? 0) <= tokens)
    .reduce<Cost | undefined>((best, entry) => (!best || (entry.tier?.size ?? 0) > (best.tier?.size ?? 0) ? entry : best), undefined)

  if (!tier) return { input: 0, cache_read: 0, cache_write: 0 }

  const micros = (dollars: number) => Math.max(0, Math.round(dollars * 1_000_000))

  return { input: micros(tier.input), cache_read: micros(tier.cache.read), cache_write: micros(tier.cache.write) }
}
