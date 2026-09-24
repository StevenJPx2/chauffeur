import type { Plugin } from "@opencode/plugin"
import type { DaemonBridge } from "./daemon.js"
import { signal } from "./model-router.js"
import { deliverContext } from "./skills.js"
import { clip } from "./text.js"

const TEXT_CODE_POINTS = 512
const ERROR_CODE_POINTS = 240
// Tools without best-practice notes answer at once; judged tools wait at most this.
const TOOL_RESULT_TIMEOUT_MS = 3_000

/**
 * Reports each tool result with a summary of its input, and delivers any
 * context the engine returns.
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

      // Two effects can name the same skill; deliver each skill once.
      const delivered = new Set<string>()

      for (const effect of effects) {
        if (effect.agent_id !== sessionID || effect.type !== "context" || effect.delivery === "prompt") continue

        const skills = effect.skills.filter((skill) => !delivered.has(skill))

        skills.forEach((skill) => delivered.add(skill))
        await deliverContext(ctx, event.sessionID, { ...effect, skills })
      }
    } catch (error) {
      // Fails open: the tool result stands unannotated.
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
