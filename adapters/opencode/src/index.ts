import type { Plugin } from "@opencode/plugin"
import { append, apply } from "./act.ts"
import { carrier } from "./carry.ts"
import { emit, installation, socketPath, type Action } from "./daemon.ts"
import { toolAfter } from "./event.ts"

type Options = { socket?: unknown; deadline_ms?: unknown }

const DEFAULT_DEADLINE_MS = 3000
const QUIET_MS = 60_000

/** Logs each kind of trouble at most once a minute. */
function warner(log: (message: string) => void, now = Date.now) {
  const last = new Map<string, number>()

  return (kind: string, message: string) => {
    const at = now()

    if (at - (last.get(kind) ?? -Infinity) < QUIET_MS) return

    last.set(kind, at)
    log(`chauffeur: ${message}`)
  }
}

/** The plugin, named after its installation so beside v1's they both load. */
export function adapter(module: string, log = console.warn): Plugin.Plugin {
  const name = installation(module)

  return {
    id: name,

    async setup(ctx) {
      const options: Options = ctx.options ?? {}
      const socket = socketPath(
        name,
        process.env,
        typeof options.socket === "string" ? options.socket : undefined
      )
      const deadlineMs =
        typeof options.deadline_ms === "number"
          ? options.deadline_ms
          : DEFAULT_DEADLINE_MS
      const warn = warner(log)
      const carried = carrier()

      const ask = async (event: Record<string, unknown>, tool: string) => {
        try {
          return await emit(socket, event, deadlineMs)
        } catch (error) {
          const reason = error instanceof Error ? error.message : String(error)

          warn("daemon", `nothing sent for ${tool}: ${reason} (${socket})`)

          return [] as Action[]
        }
      }

      await ctx.tool.hook("execute.after", async (call) => {
        const event = toolAfter(call)

        if (!event || call.status !== "completed") return

        const { result, appended, skipped } = apply(
          call.result,
          await ask(event, call.tool)
        )

        call.result = append(result, carried.take(call.id))
        carried.put(call.id, appended)

        for (const action of skipped) {
          warn(
            `skip:${action.action}`,
            `skipped ${action.action} from ${action.rule}`
          )
        }
      })
    }
  }
}

export default adapter(import.meta.url)
