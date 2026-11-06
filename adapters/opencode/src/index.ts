import type { Plugin } from "@opencode/plugin"
import { messages, type Message } from "./act.ts"
import { emit, installation, socketPath, type Action } from "./daemon.ts"
import { toolAfter } from "./event.ts"
import { fileLog, logPath } from "./log.ts"

type Options = { socket?: unknown; deadline_ms?: unknown }

const DEFAULT_DEADLINE_MS = 3000
/** The daemon's own limit; it would wait no longer for a larger one. */
const MAX_DEADLINE_MS = 30_000
const QUIET_MS = 60_000

/** The `deadline_ms` option, kept to what the daemon allows. */
function deadline(option: unknown): number {
  if (typeof option !== "number" || !Number.isFinite(option) || option < 0) {
    return DEFAULT_DEADLINE_MS
  }

  return Math.min(Math.round(option), MAX_DEADLINE_MS)
}

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

/**
 * The plugin, named after its installation so beside v1's they both load.
 * Trouble goes to `opencode.log` in the installation's state folder.
 */
export function adapter(
  module: string,
  log?: (message: string) => void
): Plugin.Plugin {
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
      const deadlineMs = deadline(options.deadline_ms)
      const warn = warner(log ?? fileLog(logPath(name, process.env)))

      const ask = async (event: Record<string, unknown>, tool: string) => {
        try {
          return await emit(socket, event, deadlineMs)
        } catch (error) {
          const reason = error instanceof Error ? error.message : String(error)

          warn("daemon", `nothing sent for ${tool}: ${reason} (${socket})`)

          return [] as Action[]
        }
      }

      const send = async (sessionID: string, message: Message) => {
        const { rule, text, description, delivery, resume } = message

        try {
          await ctx.session.synthetic({
            sessionID,
            text,
            description,
            metadata: { chauffeur: { rule } },
            delivery,
            ...(delivery === "queue" ? { resume } : {})
          })
        } catch (error) {
          const reason = error instanceof Error ? error.message : String(error)

          warn("synthetic", `nothing delivered for ${rule}: ${reason}`)
        }
      }

      await ctx.tool.hook("execute.after", async (call) => {
        const event = toolAfter(call)

        if (!event || call.status !== "completed") return

        const planned = messages(await ask(event, call.tool))

        for (const message of planned.messages) {
          await send(call.sessionID, message)
        }

        for (const action of planned.skipped) {
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
