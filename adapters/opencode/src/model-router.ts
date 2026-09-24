import { Model, type Plugin, Provider } from "@opencode/plugin/effect"
import type { SessionRetry } from "@opencode/plugin/effect/session"
import { Effect, Stream } from "effect"
import { Daemon } from "./daemon.js"
import { Host } from "./host.js"
import { signal, TEXT_CODE_POINTS, type ModelRef } from "./protocol.js"
import { clip } from "./text.js"

const MAX_SESSIONS = 256

const MAX_MODELS = 256

type HostModel = { readonly providerID: string; readonly id: string; readonly variant?: string | undefined }

type HostModelRef = Parameters<Plugin.Context["session"]["switchModel"]>[0]["model"]

/** Per-session execution state the router needs to judge a retry safely. */
class Sessions {
  private readonly toolRan = new Map<string, boolean>()
  private readonly generations = new Map<string, number>()
  private readonly active = new Set<string>()
  readonly requested = new Map<string, ModelRef>()

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
  const sessions = new Sessions()

  yield* host.session.hook("retry", (event) =>
    route(event, sessions.toolExecuted(String(event.sessionID)), sessions.current(String(event.sessionID))).pipe(
      Effect.provideService(Host, host),
      Effect.provideService(Daemon, daemon),
    ))

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

      if (event.type === "session.execution.interrupted" || event.type === "session.execution.failed" || event.type === "session.execution.succeeded") {
        return Effect.sync(() => {
          sessions.advance(event.data.sessionID, false)
          sessions.requested.delete(event.data.sessionID)
        })
      }

      const model = event.type === "session.step.ended" ? sessions.requested.get(event.data.sessionID) : undefined

      if (event.type !== "session.step.ended" || !model) return Effect.void

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
function route(event: SessionRetry, toolExecuted: boolean, valid: () => boolean): Effect.Effect<void, never, Host | Daemon> {
  const sessionID = String(event.sessionID)

  return Effect.gen(function* () {
    const host = yield* Host
    const daemon = yield* Daemon
    const models = (yield* host.model.list()).data

    if (!valid()) return

    const effects = yield* daemon.signal(signal(sessionID, {
      type: "model_error",
      model: ref(event.model),
      error_type: clip(event.error.type, TEXT_CODE_POINTS),
      status: event.error.status ?? null,
      message: clip(event.error.message, TEXT_CODE_POINTS),
      tool_executed: toolExecuted,
      available: available(models),
    }))

    // Keeping the model (`model: null`) leaves the host's own retry decision in place.
    const [next] = effects.flatMap((effect) => (effect.agent_id === sessionID && effect.type === "model" && effect.model ? [effect.model] : []))

    if (next) yield* failover(host, event, next, valid)
  }).pipe(
    // The host's own retry decision stands when the engine is unavailable.
    Effect.catch((error) => Effect.logError("chauffeur: model routing unavailable", error)),
  )
}

/** Usable models first; the engine accepts at most MAX_MODELS. */
function available(models: ReadonlyArray<HostModel & { readonly enabled: boolean; readonly status: string }>) {
  return models
    .map((model) => ({ model: ref(model), usable: model.enabled && model.status === "active" }))
    .toSorted((a, b) => Number(b.usable) - Number(a.usable))
    .slice(0, MAX_MODELS)
}

/** Switch to `next` and retry now, unless the execution ended or the selection moved on. */
function failover(host: Plugin.Context, event: SessionRetry, next: ModelRef, valid: () => boolean): Effect.Effect<void, unknown> {
  return Effect.gen(function* () {
    if (!valid()) return

    const current = (yield* host.session.get({ sessionID: event.sessionID })).model

    // A user or another hook already changed the selection while Jev
    // answered. Do not overwrite their newer choice with a stale effect.
    // No selection means the session runs on the default, the model that failed.
    if (!valid() || (current && !sameModel(ref(current), ref(event.model)))) return

    yield* host.session.switchModel({ sessionID: event.sessionID, model: hostModel(next) })

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
