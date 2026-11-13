import type { Plugin } from "@opencode/plugin/effect"
import { Duration, Effect, Fiber, Option, Schema, type Scope, Stream } from "effect"
import { Daemon } from "./daemon.js"
import { Host, type SessionID, toSessionID } from "./host.js"
import { ref } from "./model-router.js"
import { type CachePrices, type HostEffect, type Signal, signal } from "./protocol.js"
import { cacheSeconds, contextFacts, windowOf } from "./renewal.js"
import { Texts } from "./texts.js"

const MAX_SESSIONS = 256

/** How long one warming request may take before it counts as failed. */
const WARM_TIMEOUT: Duration.Input = "2 minutes"

/** What OpenCode answers a warming request with. */
export type WarmingReply = Effect.Success<ReturnType<Plugin.Context["session"]["generate"]>>

/** A warming reply's cache usage, which OpenCode reports beside its text. */
const CacheUsage = Schema.Struct({ tokens: Schema.Struct({ cache: Schema.Struct({ read: Schema.Number, write: Schema.Number }) }) })

const reportsCache = Schema.is(CacheUsage)

/**
 * A warming request that wrote more of the cache than it read rebuilt it
 * instead of keeping it: its prompt differs from the session's own, so the
 * next one would pay the same again. A reply without cache usage is no miss.
 */
export function missedCache(reply: WarmingReply): boolean {
  return reportsCache(reply) && reply.tokens.cache.read < reply.tokens.cache.write
}

/** One price tier of a model the host lists, in dollars per million tokens. */
type Cost = {
  readonly tier?: { readonly size: number } | undefined
  readonly input: number
  readonly cache: { readonly read: number; readonly write: number }
}

export type WarmingControl = {
  /** When a warming request last refreshed the session's prompt cache, in epoch milliseconds. */
  readonly lastWarmed: (sessionID: string) => number | undefined
  /**
   * Hands `listener` each session the engine asked to renew once idle, and
   * how many seconds from its turn's end, warming included, or from the
   * question to the user it waits on.
   */
  readonly expiring: (listener: ExpiryListener) => void
}

/** The agent's question to the user that a session waits on, asked at `asked`, in epoch milliseconds, as the question tool took it. */
export type PendingQuestion = { readonly id: string; readonly asked: number; readonly questions: ReadonlyArray<AskedQuestion> }

/** One question as the question tool takes it. */
export type AskedQuestion = {
  readonly question: string
  readonly header: string
  readonly options: ReadonlyArray<{ readonly label: string; readonly description: string }>
  readonly multiple: boolean
}

/** When a session's cache was last read, in epoch milliseconds, and how long after that it is renewed. */
type Expiry = { readonly since: number; readonly rest: number }

/** Told a session goes cold `afterSeconds` from now, if it stays idle, or still waits on `question`. */
export type ExpiryListener = (sessionID: SessionID, afterSeconds: number, question?: PendingQuestion) => Effect.Effect<void>

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
  let expiring: ExpiryListener = () => Effect.void

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

  /** A warming request that misses the cache, its prompt shaped differently by another plugin, stops warming and moves the `expiry` up to when the cache last read lapses. */
  const warm = (sessionID: SessionID, everySeconds: number, times: number, expiry?: Expiry) => Effect.gen(function* () {
    let touched = expiry?.since ?? Date.now()

    for (let sent = 0; sent < times; sent += 1) {
      yield* Effect.sleep(Duration.seconds(everySeconds))

      const missed = yield* host.session.generate({ sessionID, prompt: texts.current().warming.prompt }).pipe(
        Effect.timeout(WARM_TIMEOUT),
        Effect.map(missedCache),
        Effect.tap((miss) => Effect.sync(() => {
          if (miss) return

          if (!warmed.has(String(sessionID)) && warmed.size >= MAX_SESSIONS) warmed.clear()

          touched = Date.now()
          warmed.set(String(sessionID), touched)
        })),
        Effect.catch((error) => Effect.logWarning("chauffeur: a warming request failed", error).pipe(Effect.as(false))),
      )

      if (!missed) continue

      if (expiry) yield* expiring(sessionID, Math.max(0, touched + expiry.rest * 1_000 - Date.now()) / 1_000)

      return yield* Effect.logWarning("chauffeur: stopped warming a session whose warming request missed its prompt cache", { sessionID })
    }
  })

  const schedule = (sessionID: SessionID) => Effect.gen(function* () {
    const id = String(sessionID)
    const execution = executions.get(id)
    const idle = yield* idleSignal(sessionID)

    if (!idle) return

    const { plan, after } = idlePlan(yield* daemon.signal(idle), id)

    yield* stop(id)

    // A turn that began while the engine decided keeps its own cache warm.
    if (executions.get(id) !== execution) return

    if (after !== undefined) yield* expiring(sessionID, after)

    if (!plan) return

    if (loops.size >= MAX_SESSIONS) yield* Effect.forEach([...loops.keys()], stop, { discard: true })

    loops.set(id, yield* warm(sessionID, plan.every, plan.times, after === undefined ? undefined : { since: Date.now(), rest: after - plan.every * plan.times }).pipe(Effect.forkIn(scope)))
  }).pipe(
    // Warming fails open: the cache expires as it would without Chauffeur.
    Effect.catch((error) => Effect.logError("chauffeur: warming unavailable", error)),
  )

  // Any request of the session's own ends its warming; a warming request is a `generate`, not a `context`.
  yield* host.session.hook("context", (event) => stop(String(event.sessionID)))

  yield* watchQuestions((sessionID, afterSeconds, question) => expiring(sessionID, afterSeconds, question))

  yield* host.event.subscribe().pipe(
    Stream.runForEach((event) => {
      if (event.type === "session.execution.started") return started(event.data.sessionID)

      return event.type === "session.execution.succeeded" ? schedule(event.data.sessionID).pipe(Effect.forkIn(scope), Effect.asVoid) : Effect.void
    }),
    Effect.catch((error) => Effect.logError("chauffeur: warming events ended", error)),
    Effect.forkScoped,
  )

  return {
    lastWarmed: (sessionID) => warmed.get(sessionID),
    expiring: (listener) => { expiring = listener },
  }
})

/** A form OpenCode's question tool opened for the agent's question to the user. */
const QuestionForm = Schema.Struct({ kind: Schema.Literal("question") })

const isQuestion = Schema.is(QuestionForm)

/** One field of a question form: a question, its short label, and its choices. */
const QuestionField = Schema.Struct({
  title: Schema.optional(Schema.String),
  description: Schema.optional(Schema.String),
  type: Schema.optional(Schema.String),
  options: Schema.optional(Schema.Array(Schema.Struct({ label: Schema.String, description: Schema.optional(Schema.String) }))),
})

const decodeFields = Schema.decodeUnknownOption(Schema.Array(QuestionField))

/** The fields of a form OpenCode opened. */
type FormFields = Extract<Stream.Success<ReturnType<Plugin.Context["event"]["subscribe"]>>, { readonly type: "form.created" }>["data"]["form"]["fields"]

/** A question form's fields as the question tool took them; undefined for fields of another shape. */
function askedQuestions(fields: FormFields): ReadonlyArray<AskedQuestion> | undefined {
  return Option.getOrUndefined(Option.map(decodeFields(fields), (decoded) => decoded.map((field) => ({
    question: field.description ?? field.title ?? "",
    header: field.title ?? "",
    options: (field.options ?? []).map((option) => ({ label: option.label, description: option.description ?? "" })),
    multiple: field.type === "multiselect",
  }))))
}

/**
 * Reports each question the agent asks the user to the engine, as its turn
 * pauses on it: the context's cache then lapses while the user is away, and
 * the answer would re-read it cold. When the engine asks to renew it once
 * the cache expires, `tell` hears of it with the question.
 */
function watchQuestions(tell: ExpiryListener): Effect.Effect<void, never, Host | Daemon | Scope.Scope> {
  return Effect.gen(function* () {
    const host = yield* Host
    const daemon = yield* Daemon
    const scope = yield* Effect.scope

    const asked = (sessionID: SessionID, question: PendingQuestion) => Effect.gen(function* () {
      const idle = yield* idleSignal(sessionID, true)

      if (!idle) return

      const { after } = idlePlan(yield* daemon.signal(idle), String(sessionID))

      if (after !== undefined) yield* tell(sessionID, after, question)
    }).pipe(Effect.catch((error) => Effect.logError("chauffeur: a question to the user was not reported", error)))

    yield* host.event.subscribe().pipe(
      Stream.runForEach((event) => {
        if (event.type !== "form.created" || !isQuestion(event.data.form.metadata)) return Effect.void

        const { form } = event.data
        const questions = askedQuestions(form.fields)

        return questions ? asked(toSessionID(form.sessionID), { id: String(form.id), asked: Date.now(), questions }).pipe(Effect.forkIn(scope), Effect.asVoid) : Effect.void
      }),
      Effect.catch((error) => Effect.logError("chauffeur: question events ended", error)),
      Effect.forkScoped,
    )
  })
}

/**
 * What a finished turn, or one `awaitingAnswer` to the agent's question,
 * left in the cache, for the engine: the session's model, context size and
 * prices. While a question waits, the step that asked it is not measured
 * yet, so the size is the latest measured reply before it. Undefined for a
 * subagent's session, one in another location, or a context without a
 * measured reply.
 */
function idleSignal(sessionID: SessionID, awaitingAnswer = false): Effect.Effect<Signal | undefined, unknown, Host> {
  return Effect.gen(function* () {
    const host = yield* Host
    const session = yield* host.session.get({ sessionID })

    // Only the session's own location reports it.
    if (session.parentID !== undefined || String(session.location.directory) !== String(host.location.directory)) return undefined

    const facts = contextFacts(yield* host.session.context({ sessionID }), Date.now())

    if (!facts) return undefined

    const model = session.model ?? facts.model
    const models = (yield* host.model.list()).data
    const listed = models.find((entry) => entry.providerID === model.providerID && entry.id === model.id)

    return signal(String(sessionID), {
      type: "context_idle",
      model: ref(model),
      context_tokens: facts.tokens,
      context_window: windowOf(models, ref(model)) ?? 0,
      awaiting_answer: awaitingAnswer,
      prices: pricesAt(listed?.cost ?? [], facts.tokens),
      cache_seconds: cacheSeconds(listed, model.variant),
    })
  })
}

/** The engine's warming for session `id`, and the seconds after which to renew it if still idle, if it asked for either. */
function idlePlan(effects: ReadonlyArray<HostEffect>, id: string) {
  const warm = effects.find((effect) => effect.type === "warm" && effect.agent_id === id)
  const renew = effects.find((effect) => effect.type === "renew_when_idle" && effect.agent_id === id)

  return {
    plan: warm?.type === "warm" && warm.times > 0 && warm.every_seconds > 0 ? { every: warm.every_seconds, times: warm.times } : undefined,
    after: renew?.type === "renew_when_idle" ? renew.after_seconds : undefined,
  }
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
