import { Model, type Plugin, Provider } from "@opencode/plugin/effect"
import type { SessionRetry } from "@opencode/plugin/effect/session"
import { Effect, Stream } from "effect"
import { Daemon } from "./daemon.js"
import { Host, type SessionID } from "./host.js"
import { signal, TEXT_CODE_POINTS, type ModelRef } from "./protocol.js"
import { deliverContext } from "./skills.js"
import { clip } from "./text.js"
import { fill, type HostTexts, Texts } from "./texts.js"

const MAX_SESSIONS = 256

const MAX_MODELS = 256

type HostEvent = Stream.Success<ReturnType<Plugin.Context["event"]["subscribe"]>>

type ExecutionEnd = Extract<HostEvent, { readonly type: "session.execution.interrupted" | "session.execution.failed" | "session.execution.succeeded" }>

const EXECUTION_ENDED = new Set(["session.execution.interrupted", "session.execution.failed", "session.execution.succeeded"])

const executionEnded = (event: HostEvent): event is ExecutionEnd => EXECUTION_ENDED.has(event.type)

/** How much of a model's error a note to the agent quotes. */
const ERROR_CODE_POINTS = 300

type HostModel = { readonly providerID: string; readonly id: string; readonly variant?: string | undefined }

type HostModelRef = Parameters<Plugin.Context["session"]["switchModel"]>[0]["model"]

/** Per-session execution state the router needs to judge a retry safely. */
class Sessions {
  private readonly toolRan = new Map<string, boolean>()
  private readonly generations = new Map<string, number>()
  private readonly active = new Set<string>()
  readonly requested = new Map<string, ModelRef>()
  /** The model Chauffeur switched a session to, until it completes a step. */
  readonly switched = new Map<string, ModelRef>()

  switchedTo(sessionID: string, model: ModelRef): void {
    if (!this.switched.has(sessionID) && this.switched.size >= MAX_SESSIONS) this.switched.clear()

    this.switched.set(sessionID, model)
  }

  advance(sessionID: string, running: boolean): void {
    if (!this.generations.has(sessionID) && this.generations.size >= MAX_SESSIONS) this.generations.clear()

    this.generations.set(sessionID, (this.generations.get(sessionID) ?? 0) + 1)

    if (running && !this.active.has(sessionID) && this.active.size >= MAX_SESSIONS) this.active.clear()

    if (running) this.active.add(sessionID)
    else this.active.delete(sessionID)
  }

  markTool(sessionID: string, ran: boolean): void {
    if (!this.toolRan.has(sessionID) && this.toolRan.size >= MAX_SESSIONS) this.toolRan.clear()

    this.toolRan.set(sessionID, ran)
  }

  request(sessionID: string, model: ModelRef): void {
    if (!this.requested.has(sessionID) && this.requested.size >= MAX_SESSIONS) this.requested.clear()

    this.requested.set(sessionID, model)
  }

  /** The execution ended; the model Chauffeur switched it to, if no step completed on it. */
  end(sessionID: string): ModelRef | undefined {
    const switched = this.switched.get(sessionID)

    this.advance(sessionID, false)
    this.requested.delete(sessionID)
    this.switched.delete(sessionID)

    return switched
  }

  /** A step completed: the model it ran on, now proven to serve the session. */
  stepEnded(sessionID: string): ModelRef | undefined {
    this.switched.delete(sessionID)

    return this.requested.get(sessionID)
  }

  toolExecuted(sessionID: string): boolean {
    return this.toolRan.get(sessionID) ?? false
  }

  /** True while the execution that was current when this is called is still running. */
  current(sessionID: string): () => boolean {
    const generation = this.generations.get(sessionID)

    return () => this.active.has(sessionID) && this.generations.get(sessionID) === generation
  }
}

/**
 * Senses model errors, successes, and tool runs, and applies the engine's
 * model effects. All failover policy lives in the Chauffeur engine.
 */
export const installModelRouter = Effect.gen(function* () {
  const host = yield* Host
  const daemon = yield* Daemon
  const texts = yield* Texts
  const sessions = new Sessions()

  yield* host.session.hook("retry", (event) => {
    const sessionID = String(event.sessionID)

    // The hook reports this failure itself; a later end of the execution is not news.
    sessions.switched.delete(sessionID)

    return route(event, sessions.toolExecuted(sessionID), sessions.current(sessionID), (next) => sessions.switchedTo(sessionID, next)).pipe(
      Effect.provideService(Host, host),
      Effect.provideService(Daemon, daemon),
    )
  })

  // A tool in an earlier model request is already settled. Only tools run
  // during this request can make retrying its failed step unsafe.
  yield* host.session.hook("model.request", (event) => Effect.sync(() => {
    if (event.kind !== "primary") return

    sessions.markTool(String(event.sessionID), false)
    sessions.request(String(event.sessionID), ref(event.model))
  }))

  yield* host.tool.hook("execute.after", (event) => Effect.sync(() => sessions.markTool(String(event.sessionID), true)))

  yield* host.event.subscribe().pipe(
    Stream.runForEach((event) => {
      if (event.type === "session.execution.started") {
        return Effect.sync(() => {
          sessions.advance(event.data.sessionID, true)
          sessions.markTool(event.data.sessionID, false)
        })
      }

      if (executionEnded(event)) {
        const sessionID = event.data.sessionID
        const switched = sessions.end(sessionID)

        // The model Chauffeur switched to failed before any step completed,
        // in a way the retry hook never saw: report it, or the agent stops.
        if (event.type !== "session.execution.failed" || !switched) return Effect.void

        return followUp(sessionID, switched, event.data.error, texts.current(), (next) => sessions.switchedTo(sessionID, next)).pipe(
          Effect.provideService(Host, host),
          Effect.provideService(Daemon, daemon),
          Effect.forkScoped,
          Effect.asVoid,
        )
      }

      if (event.type !== "session.step.ended") return Effect.void

      const model = sessions.stepEnded(event.data.sessionID)

      if (!model) return Effect.void

      return daemon.signal(signal(event.data.sessionID, { type: "model_succeeded", model })).pipe(
        Effect.catch((error) => Effect.logError("chauffeur: signal dropped", error)),
        Effect.forkScoped,
        Effect.asVoid,
      )
    }),
    Effect.catch((error) => Effect.logError("chauffeur: model routing events ended", error)),
    Effect.forkScoped,
  )
})

/** Report a failed model request and apply the engine's model decision. */
function route(event: SessionRetry, toolExecuted: boolean, valid: () => boolean, switched: (next: ModelRef) => void): Effect.Effect<void, never, Host | Daemon> {
  return Effect.gen(function* () {
    if (!valid()) return

    const next = yield* report(String(event.sessionID), ref(event.model), event.error, toolExecuted, valid)

    // Keeping the model (`model: null`) leaves the host's own retry decision in place.
    if (next) yield* failover(event, next, valid, switched)
  }).pipe(
    // The host's own retry decision stands when the engine is unavailable.
    Effect.catch((error) => Effect.logError("chauffeur: model routing unavailable", error)),
  )
}

type ModelFailure = { readonly type: string; readonly status?: number | undefined; readonly message: string }

/**
 * Report a failed model to the engine. The model to move to; `null` when the
 * engine keeps the model; `undefined` when the failure is not the router's.
 */
function report(sessionID: string, model: ModelRef, error: ModelFailure, toolExecuted: boolean, valid: () => boolean = () => true): Effect.Effect<ModelRef | null | undefined, unknown, Host | Daemon> {
  return Effect.gen(function* () {
    const host = yield* Host
    const daemon = yield* Daemon
    const models = (yield* host.model.list()).data

    if (!valid()) return undefined

    const effects = yield* daemon.signal(signal(sessionID, {
      type: "model_error",
      model,
      error_type: clip(error.type, TEXT_CODE_POINTS),
      status: error.status ?? null,
      message: clip(error.message, TEXT_CODE_POINTS),
      tool_executed: toolExecuted,
      available: available(models),
    }))

    const decision = effects.find((effect) => effect.agent_id === sessionID && effect.type === "model")

    return decision?.type === "model" ? decision.model : undefined
  })
}

/**
 * The model Chauffeur switched to failed and ended the execution. Move on to
 * the engine's next choice and wake the agent, or say that none is left.
 */
function followUp(sessionID: SessionID, failed: ModelRef, error: ModelFailure, texts: HostTexts, switched: (next: ModelRef) => void): Effect.Effect<void, never, Host | Daemon> {
  return Effect.gen(function* () {
    const host = yield* Host
    // A new turn starts from the session's history, so no step is repeated.
    const next = yield* report(String(sessionID), failed, error, false)
    const current = (yield* host.session.get({ sessionID })).model

    // Not the router's failure, or the user already chose another model.
    if (next === undefined || (current && !sameModel(ref(current), failed))) return

    const reason = clip(`${error.type}: ${error.message}`, ERROR_CODE_POINTS)

    if (next) {
      yield* host.session.switchModel({ sessionID, model: hostModel(next) })
      switched(next)
    }

    yield* deliverContext(sessionID, {
      type: "context",
      agent_id: String(sessionID),
      delivery: next ? "resume" : "wait",
      label: texts.model_router.label,
      skills: [],
      text: next
        ? fill(texts.model_router.continue, { model: modelKey(next), failed: modelKey(failed), error: reason })
        : fill(texts.model_router.stranded, { model: modelKey(failed), error: reason }),
    }, texts)
  }).pipe(Effect.catch((error) => Effect.logError("chauffeur: model follow-up failed", error)))
}

function modelKey(model: ModelRef): string {
  return `${model.provider}/${model.model}${model.variant ? `#${model.variant}` : ""}`
}

/** Usable models first; the engine accepts at most MAX_MODELS. */
function available(models: ReadonlyArray<HostModel & { readonly enabled: boolean; readonly status: string }>) {
  return models
    .map((model) => ({ model: ref(model), usable: model.enabled && model.status === "active" }))
    .toSorted((a, b) => Number(b.usable) - Number(a.usable))
    .slice(0, MAX_MODELS)
}

/** Switch to `next` and retry now, unless the execution ended or the selection moved on. */
function failover(event: SessionRetry, next: ModelRef, valid: () => boolean, switched: (next: ModelRef) => void): Effect.Effect<void, unknown, Host> {
  return Effect.gen(function* () {
    const host = yield* Host

    if (!valid()) return

    const current = (yield* host.session.get({ sessionID: event.sessionID })).model

    // A user or another hook already changed the selection while Jev
    // answered. Do not overwrite their newer choice with a stale effect.
    // No selection means the session runs on the default, the model that failed.
    if (!valid() || (current && !sameModel(ref(current), ref(event.model)))) return

    yield* host.session.switchModel({ sessionID: event.sessionID, model: hostModel(next) })
    switched(next)

    if (valid()) event.decision = { retry: true, delay: 0 }
  })
}

export function sameModel(left: ModelRef, right: ModelRef): boolean {
  return left.provider === right.provider && left.model === right.model && left.variant === right.variant
}

/** The host's model as Chauffeur's reference; OpenCode's `default` variant is no variant. */
export function ref(model: HostModel): ModelRef {
  return {
    provider: model.providerID,
    model: model.id,
    variant: model.variant === "default" ? undefined : model.variant || undefined,
  }
}

/** Chauffeur's reference as the host's, carrying its thinking variant. */
export function hostModel(model: ModelRef): HostModelRef {
  return {
    providerID: Provider.ID.make(model.provider),
    id: Model.ID.make(model.model),
    variant: model.variant === undefined ? undefined : Model.VariantID.make(model.variant),
  }
}
