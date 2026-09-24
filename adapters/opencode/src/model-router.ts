import type { Plugin } from "@opencode/plugin"
import type { SessionRetry } from "@opencode/plugin/promise/session"
import type { DaemonBridge } from "./daemon.js"
import type { ModelRef, Signal, SignalKind } from "./protocol.js"
import { clip } from "./text.js"

const MAX_SESSIONS = 256
const MAX_MODELS = 256
const TEXT_CODE_POINTS = 512

type HostModel = { providerID: string; id: string }

/**
 * Senses model errors, successes, and tool runs, and applies the engine's
 * model effects. All failover policy lives in the Chauffeur engine.
 */
export async function installModelRouter(
  ctx: Plugin.Context,
  daemon: DaemonBridge,
): Promise<() => Promise<void>> {
  const toolRan = new Map<string, boolean>()
  const requested = new Map<string, ModelRef>()
  const generations = new Map<string, number>()
  const active = new Set<string>()
  const controller = new AbortController()
  const advance = (sessionID: string, running: boolean): void => {
    if (!generations.has(sessionID) && generations.size >= MAX_SESSIONS) generations.clear()
    generations.set(sessionID, (generations.get(sessionID) ?? 0) + 1)
    if (running) active.add(sessionID)
    else active.delete(sessionID)
  }
  const markTool = (sessionID: string, ran: boolean): void => {
    if (!toolRan.has(sessionID) && toolRan.size >= MAX_SESSIONS) toolRan.clear()

    toolRan.set(sessionID, ran)
  }

  const retry = await ctx.session.hook("retry", (event) => {
    const id = String(event.sessionID)
    const generation = generations.get(id)

    return route(ctx, daemon, event, toolRan.get(id) ?? false, () =>
      active.has(id) && generations.get(id) === generation)
  })
  // A tool in an earlier model request is already settled. Only tools run
  // during this request can make retrying its failed step unsafe.
  const request = await ctx.session.hook("model.request", (event) => {
    if (event.kind !== "primary") return

    markTool(String(event.sessionID), false)
    if (!requested.has(String(event.sessionID)) && requested.size >= MAX_SESSIONS) requested.clear()
    requested.set(String(event.sessionID), ref(event.model))
  })
  const tool = await ctx.tool.hook("execute.after", (event) => {
    markTool(String(event.sessionID), true)
  })

  void watchEvents(ctx, daemon, controller, requested, markTool, advance)

  return async () => {
    controller.abort()
    await retry.dispose()
    await request.dispose()
    await tool.dispose()
    toolRan.clear()
    requested.clear()
    generations.clear()
    active.clear()
  }
}

async function watchEvents(
  ctx: Plugin.Context,
  daemon: DaemonBridge,
  controller: AbortController,
  requested: Map<string, ModelRef>,
  markTool: (id: string, ran: boolean) => void,
  advance: (id: string, running: boolean) => void,
): Promise<void> {
  try {
    for await (const event of ctx.event.subscribe({ signal: controller.signal })) {
      const id = (event.data as Record<string, unknown>).sessionID

      if (typeof id !== "string") continue
      if (event.type === "session.execution.started") {
        advance(id, true)
        markTool(id, false)
      }
      if (event.type === "session.execution.interrupted" || event.type === "session.execution.failed" || event.type === "session.execution.succeeded") {
        advance(id, false)
        requested.delete(id)
      }
      if (event.type === "session.step.ended") {
        const model = requested.get(id)

        if (model) send(daemon, signal(id, { type: "model_succeeded", model }))
      }
    }
  } catch (error) {
    if (!controller.signal.aborted) console.error(`[chauffeur] model routing events ended: ${String(error)}`)
  }
}

/** Report a failed model request and apply the engine's model decision. */
async function route(ctx: Plugin.Context, daemon: DaemonBridge, event: SessionRetry, toolExecuted: boolean, valid: () => boolean): Promise<void> {
  const sessionID = String(event.sessionID)

  try {
    const models = (await ctx.model.list()).data
    if (!valid()) return
    const effects = await daemon.signal(signal(sessionID, {
      type: "model_error",
      model: ref(event.model),
      error_type: clip(event.error.type, TEXT_CODE_POINTS),
      status: typeof event.error.status === "number" ? event.error.status : null,
      message: clip(event.error.message, TEXT_CODE_POINTS),
      tool_executed: toolExecuted,
      // Usable models first; the engine accepts at most MAX_MODELS.
      available: models
        .map((model) => ({ model: ref(model), usable: model.enabled && model.status === "active" }))
        .sort((a, b) => Number(b.usable) - Number(a.usable))
        .slice(0, MAX_MODELS),
    }))
    if (!valid()) return

    for (const effect of effects) {
      if (effect.agent_id !== sessionID) continue

      // Keeping the model (`model: null`) leaves the host's own retry decision in place.
      if (effect.type === "model" && effect.model) {
        if (!valid()) return
        const current = (await ctx.session.get({ sessionID: event.sessionID })).model

        // A user or another hook already changed the selection while Jev
        // answered. Do not overwrite their newer choice with a stale effect.
        if (!valid() || !current || !sameModel(ref(current), ref(event.model))) return
        await ctx.session.switchModel({ sessionID: event.sessionID, model: hostModel(effect.model) })
        if (valid()) event.decision = { retry: true, delay: 0 }
      }
    }
  } catch (error) {
    // The host's own retry decision stands when the engine is unavailable.
    console.error(`[chauffeur] model routing unavailable: ${String(error)}`)
  }
}

export function sameModel(left: ModelRef, right: ModelRef): boolean {
  return left.provider === right.provider && left.model === right.model && left.variant === right.variant
}

export function signal(agentID: string, kind: SignalKind): Signal {
  return { agent_id: agentID, at: Math.floor(Date.now() / 1_000), kind }
}

function send(daemon: DaemonBridge, value: Signal): void {
  daemon.signal(value).catch((error: unknown) => {
    console.error(`[chauffeur] signal dropped: ${String(error)}`)
  })
}

/** The host's model as Chauffeur's reference; OpenCode's `default` variant is no variant. */
export function ref(model: HostModel & { variant?: string | undefined }): ModelRef {
  const variant = model.variant && model.variant !== "default" ? model.variant : undefined

  return { provider: model.providerID, model: model.id, ...(variant ? { variant } : {}) }
}

type HostModelRef = Parameters<Plugin.Context["session"]["switchModel"]>[0]["model"]

/** Chauffeur's reference as the host's, carrying its thinking variant. */
export function hostModel(model: ModelRef): HostModelRef {
  // Safe: variants come from the engine's tier tables, named as OpenCode names them.
  const variant = model.variant as HostModelRef["variant"]

  return { providerID: model.provider, id: model.model, ...(variant ? { variant } : {}) }
}
