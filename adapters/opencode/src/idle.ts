import type { Plugin } from "@opencode/plugin"
import type { DaemonBridge } from "./daemon.js"
import { signal } from "./model-router.js"
import { deliverContext } from "./skills.js"

/**
 * Reports finished turns and delivers any context the engine returns, which
 * either wakes the idle agent or waits for its next turn.
 */
export function installIdle(ctx: Plugin.Context, daemon: DaemonBridge): () => void {
  const controller = new AbortController()

  void (async () => {
    try {
      for await (const event of ctx.event.subscribe({ signal: controller.signal })) {
        // A finished turn; an interrupted one means the user stopped the agent.
        if (event.type !== "session.execution.succeeded" && event.type !== "session.execution.failed") continue

        await reportTurnEnd(ctx, daemon, event.data.sessionID)
      }
    } catch (error) {
      if (!controller.signal.aborted) console.error(`[chauffeur] idle events ended: ${String(error)}`)
    }
  })()

  return () => controller.abort()
}

async function reportTurnEnd(ctx: Plugin.Context, daemon: DaemonBridge, sessionID: Parameters<Plugin.Context["session"]["synthetic"]>[0]["sessionID"]): Promise<void> {
  try {
    const effects = await daemon.signal(signal(String(sessionID), { type: "turn_end" }))

    for (const effect of effects) {
      if (effect.agent_id !== String(sessionID) || effect.type !== "context" || effect.delivery === "prompt") continue

      await deliverContext(ctx, sessionID, effect)
    }
  } catch (error) {
    // Fails open: nothing is delivered.
    console.error(`[chauffeur] turn end not reported: ${String(error)}`)
  }
}
