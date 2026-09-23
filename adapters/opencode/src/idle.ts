import type { Plugin } from "@opencode/plugin"
import type { DaemonBridge } from "./daemon.js"
import { signal } from "./model-router.js"
import { deliverSkills } from "./skills.js"

/**
 * Reports finished turns and delivers the engine's reminders, which resume
 * the idle agent, and skills, which wait for the next turn. Tool results already reach
 * the engine through the model-router sensor.
 */
export function installIdle(ctx: Plugin.Context, daemon: DaemonBridge): () => void {
  const controller = new AbortController()

  void (async () => {
    try {
      for await (const event of ctx.event.subscribe({ signal: controller.signal })) {
        // A finished turn; an interrupted one means the user stopped the agent.
        if (event.type !== "session.execution.succeeded" && event.type !== "session.execution.failed") continue

        await remind(ctx, daemon, event.data.sessionID)
      }
    } catch (error) {
      if (!controller.signal.aborted) console.error(`[chauffeur] idle events ended: ${String(error)}`)
    }
  })()

  return () => controller.abort()
}

async function remind(ctx: Plugin.Context, daemon: DaemonBridge, sessionID: Parameters<Plugin.Context["session"]["synthetic"]>[0]["sessionID"]): Promise<void> {
  try {
    const effects = await daemon.signal(signal(String(sessionID), { type: "turn_end" }))

    for (const effect of effects) {
      if (effect.agent_id !== String(sessionID)) continue

      if (effect.type === "remind") {
        await ctx.session.synthetic({ sessionID, text: effect.text, description: `Chauffeur ${effect.rule_id}`, resume: true })
      } else if (effect.type === "attach_skills") {
        // A finished turn keeps the skill for the next one instead of redoing work.
        await deliverSkills(ctx, sessionID, effect.skills, false)
      }
    }
  } catch (error) {
    // Idle nudges fail open: the reminder is skipped.
    console.error(`[chauffeur] idle reminder skipped: ${String(error)}`)
  }
}
