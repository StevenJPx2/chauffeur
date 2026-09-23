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
  const controller = new AbortController()
  const markTool = (sessionID: string, ran: boolean): void => {
    if (!toolRan.has(sessionID) && toolRan.size >= MAX_SESSIONS) toolRan.clear()

    toolRan.set(sessionID, ran)
  }

  const retry = await ctx.session.hook("retry", (event) => route(ctx, daemon, event, toolRan.get(String(event.sessionID)) ?? false))
  const response = await ctx.session.hook("http.response", (event) => {
    if (!event.response.ok || event.kind !== "primary") return

    send(daemon, signal(String(event.sessionID), { type: "model_succeeded", model: ref(event.model) }))
  })
  const tool = await ctx.tool.hook("execute.after", (event) => {
    markTool(String(event.sessionID), true)
  })

  void (async () => {
    try {
      for await (const event of ctx.event.subscribe({ signal: controller.signal })) {
        const id = (event.data as Record<string, unknown>).sessionID

        if (typeof id !== "string") continue
        if (event.type === "session.execution.started") markTool(id, false)
      }
    } catch (error) {
      if (!controller.signal.aborted) console.error(`[chauffeur] model routing events ended: ${String(error)}`)
    }
  })()

  return async () => {
    controller.abort()
    await retry.dispose()
    await response.dispose()
    await tool.dispose()
    toolRan.clear()
  }
}

/** Report a failed model request and apply the engine's model decision. */
async function route(ctx: Plugin.Context, daemon: DaemonBridge, event: SessionRetry, toolExecuted: boolean): Promise<void> {
  const sessionID = String(event.sessionID)

  try {
    const models = (await ctx.model.list()).data
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

    for (const effect of effects) {
      if (effect.agent_id !== sessionID) continue

      // keep_model leaves the host's own retry decision in place.
      if (effect.type === "switch_model") {
        await ctx.session.switchModel({
          sessionID: event.sessionID,
          model: { providerID: effect.model.provider, id: effect.model.model },
        })
        event.decision = { retry: true, delay: 0 }
      }
    }
  } catch (error) {
    // The host's own retry decision stands when the engine is unavailable.
    console.error(`[chauffeur] model routing unavailable: ${String(error)}`)
  }
}

export function signal(agentID: string, kind: SignalKind): Signal {
  return { agent_id: agentID, at: Math.floor(Date.now() / 1_000), kind }
}

function send(daemon: DaemonBridge, value: Signal): void {
  daemon.signal(value).catch((error: unknown) => {
    console.error(`[chauffeur] signal dropped: ${String(error)}`)
  })
}

function ref(model: HostModel): ModelRef {
  return { provider: model.providerID, model: model.id }
}

