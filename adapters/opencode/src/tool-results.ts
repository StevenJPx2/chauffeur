import type { Plugin } from "@opencode/plugin"
import type { DaemonBridge } from "./daemon.js"
import { signal } from "./model-router.js"
import { deliverSkills } from "./skills.js"
import { clip } from "./text.js"

const TEXT_CODE_POINTS = 512
const ERROR_CODE_POINTS = 240
// Tools without best-practice notes answer at once; judged tools wait at most this.
const TOOL_RESULT_TIMEOUT_MS = 3_000

/**
 * Reports each tool result with a summary of its input, and steers the
 * running turn with any misuse nudge or better-fitting skill the engine returns.
 */
export async function installToolResults(ctx: Plugin.Context, daemon: DaemonBridge): Promise<() => Promise<void>> {
  const hook = await ctx.tool.hook("execute.after", async (event) => {
    const sessionID = String(event.sessionID)

    try {
      const effects = await daemon.signal(signal(sessionID, {
        type: "tool_result",
        tool: clip(event.tool, TEXT_CODE_POINTS),
        ok: event.status === "completed",
        input: summarize(event.input),
        error: event.status === "error" ? clip(event.error.message, ERROR_CODE_POINTS) : "",
      }), TOOL_RESULT_TIMEOUT_MS)

      // A misuse hand-over and a drift pick can name the same skill; deliver it once.
      const skills = new Set<string>()

      for (const effect of effects) {
        if (effect.agent_id !== sessionID) continue

        if (effect.type === "nudge") {
          await ctx.session.synthetic({ sessionID: event.sessionID, text: effect.text, description: "Chauffeur tool check", delivery: "steer" })
        } else if (effect.type === "attach_skills") {
          for (const skill of effect.skills) skills.add(skill)
        }
      }

      if (skills.size > 0) await deliverSkills(ctx, event.sessionID, [...skills], true)
    } catch (error) {
      // Misuse checks fail open: the tool result stands unannotated.
      console.error(`[chauffeur] tool result not reported: ${String(error)}`)
    }
  })

  return () => hook.dispose()
}

function summarize(input: unknown): string {
  try {
    return clip(JSON.stringify(input) ?? "", TEXT_CODE_POINTS)
  } catch {
    return "unserializable input"
  }
}
