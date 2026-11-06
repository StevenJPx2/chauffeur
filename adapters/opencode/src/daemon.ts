import { createConnection } from "node:net"
import { tmpdir } from "node:os"
import { basename, dirname, join } from "node:path"
import { fileURLToPath } from "node:url"

export type Action = {
  readonly rule?: string
  readonly action: string
  readonly payload?: unknown
}

type Env = Readonly<Record<string, string | undefined>>

/**
 * The installation the adapter ships in: `share/<name>/opencode/server.js`
 * belongs to `<name>`. Anywhere else, `chauffeur`.
 */
export function installation(module: string): string {
  const folder = dirname(dirname(fileURLToPath(module)))

  return basename(dirname(folder)) === "share" ? basename(folder) : "chauffeur"
}

/** The daemon's socket, found as the daemon finds it. */
export function socketPath(name: string, env: Env, option?: string): string {
  const set = (value: string | undefined) => (value ? value : undefined)
  const runtime = set(env.XDG_RUNTIME_DIR) ?? tmpdir()

  return option ?? set(env.CHAUFFEUR_SOCKET) ?? join(runtime, name, "daemon.sock")
}

/**
 * Sends one event with `chauffeur.emit` and resolves with the actions the
 * rules returned. Rejects when the daemon cannot be reached, answers with an
 * error, closes the connection without answering, or has not answered half a
 * second after the rules' deadline.
 */
export function emit(
  socket: string,
  event: Record<string, unknown>,
  deadlineMs: number
): Promise<Action[]> {
  const request = {
    jsonrpc: "2.0",
    id: 1,
    method: "chauffeur.emit",
    params: { event, deadline_ms: deadlineMs }
  }

  return new Promise((resolve, reject) => {
    const connection = createConnection(socket)
    let received = ""
    let answered = false

    const fail = (error: Error) => {
      connection.destroy()
      reject(error)
    }

    connection.setTimeout(deadlineMs + 500, () => {
      fail(new Error(`no answer within ${deadlineMs + 500} ms`))
    })

    connection.on("error", fail)
    connection.on("connect", () => connection.write(`${JSON.stringify(request)}\n`))

    // A daemon that stops mid-request closes cleanly, which also clears the
    // timeout: without this the call would wait forever.
    connection.on("close", () => {
      if (!answered) {
        reject(new Error("the daemon closed the connection without answering"))
      }
    })

    connection.on("data", (data) => {
      received += data.toString("utf8")
      const end = received.indexOf("\n")

      if (end === -1) return

      answered = true
      connection.end()

      try {
        resolve(actions(JSON.parse(received.slice(0, end))))
      } catch (error) {
        reject(error instanceof Error ? error : new Error(String(error)))
      }
    })
  })
}

function actions(reply: {
  result?: { actions?: unknown }
  error?: { message?: string }
}) {
  if (reply.error)
    throw new Error(reply.error.message ?? "the daemon refused the event")

  const found = reply.result?.actions

  if (!Array.isArray(found)) throw new Error("the daemon's reply has no actions")

  return found as Action[]
}
