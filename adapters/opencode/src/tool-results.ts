import type { Plugin } from "@opencode/plugin"
import type { DaemonBridge } from "./daemon.js"
import type { ExposureControl } from "./exposure.js"
import type { CatalogEntry } from "./protocol.js"
import { signal } from "./model-router.js"
import { deliverContext } from "./skills.js"
import { clip } from "./text.js"

const TEXT_CODE_POINTS = 512
const ERROR_CODE_POINTS = 240
const TOOL_RESULT_TIMEOUT_MS = 3_000
const RECOVERY_TIMEOUT_MS = 8_000
const MAX_CANDIDATES = 4

/** Report tool results and deliver context or verified hidden-tool reveals. */
export async function installToolResults(ctx: Plugin.Context, daemon: DaemonBridge, exposure: ExposureControl): Promise<() => Promise<void>> {
  const hook = await ctx.tool.hook("execute.after", async (event) => {
    const sessionID = String(event.sessionID)

    try {
      const input = summarize(event.input)
      const error = event.status === "error" ? clip(event.error.message, ERROR_CODE_POINTS) : ""
      const output = event.status === "completed" ? summarize(event.result.content ?? event.result.output) : ""
      const evidence = clip(`${error} ${output}`.trim(), TEXT_CODE_POINTS)
      const missing = /(?:unknown|missing|unavailable|not found|no matching|not registered|cannot call|not available).{0,50}(?:tool|function)|(?:tool|function).{0,50}(?:unknown|missing|unavailable|not found|not registered|not available)/i.test(evidence)
      const history = missing ? await ctx.session.context({ sessionID: event.sessionID }) : []
      const lastUser = history.filter((message) => message.type === "user").at(-1)
      const userRequest = lastUser?.type === "user" ? clip(lastUser.text, TEXT_CODE_POINTS) : ""
      const catalog = missing ? await exposure.candidates(event.sessionID) : []
      const candidates = matching(catalog, `${input} ${evidence} ${userRequest}`)
      const effects = await daemon.signal(signal(sessionID, {
        type: "tool_result",
        tool: clip(event.tool, TEXT_CODE_POINTS),
        ok: event.status === "completed",
        input, error, user_request: userRequest, evidence, candidates,
      }), candidates.length > 0 ? RECOVERY_TIMEOUT_MS : TOOL_RESULT_TIMEOUT_MS)

      const delivered = new Set<string>()

      for (const effect of effects) {
        if (effect.agent_id === sessionID && effect.type === "tools" && effect.reveal.length > 0) {
          await exposure.reveal(event.sessionID, effect.reveal)
        }
        if (effect.agent_id !== sessionID || effect.type !== "context" || effect.delivery !== "prompt") continue

        const skills = effect.skills.filter((skill) => !delivered.has(skill))

        skills.forEach((skill) => delivered.add(skill))
        await deliverContext(ctx, event.sessionID, { ...effect, skills })
      }
    } catch (error) {
      console.error(`[chauffeur] tool result not reported: ${String(error)}`)
    }
  })

  return () => hook.dispose()
}

/** Lexical matches narrow the registered hidden catalog; Jev judges need. */
function matching(catalog: ReadonlyArray<CatalogEntry>, text: string): CatalogEntry[] {
  const words = new Set(text.toLowerCase().match(/[a-z][a-z0-9]{3,}/g) ?? [])
  return catalog.filter((tool) => {
    const names = tool.id.toLowerCase().split(/[_-]/)
    return names.some((name) => name.length >= 4 && words.has(name))
      || tool.description.toLowerCase().split(/\W+/).filter((word) => word.length >= 5 && words.has(word)).length >= 2
  }).slice(0, MAX_CANDIDATES)
}

function summarize(input: unknown): string {
  try {
    return clip(JSON.stringify(input) ?? "", TEXT_CODE_POINTS)
  } catch {
    return "unserializable input"
  }
}
