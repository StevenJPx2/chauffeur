import { mkdtempSync, rmSync } from "node:fs"
import { createServer, type Server } from "node:net"
import { tmpdir } from "node:os"
import { join } from "node:path"

export type Request = {
  method: string
  params: { event: unknown; deadline_ms: number }
}

/**
 * A daemon on a real socket that answers each request line with `reply`, or
 * not at all when `reply` returns undefined.
 */
export async function standIn(reply: (request: Request) => unknown) {
  const folder = mkdtempSync(join(tmpdir(), "cho-"))
  const socket = join(folder, "s")
  const requests: Request[] = []

  const server: Server = createServer((connection) => {
    let received = ""

    connection.on("data", (data) => {
      received += data.toString("utf8")
      const end = received.indexOf("\n")

      if (end === -1) return

      const request = JSON.parse(received.slice(0, end))
      requests.push(request)
      const answer = reply(request)

      if (answer !== undefined) {
        connection.write(
          `${JSON.stringify({ jsonrpc: "2.0", id: request.id, ...answer })}\n`
        )
      }
    })
  })

  await new Promise<void>((done) => server.listen(socket, done))

  const close = async () => {
    server.close()
    rmSync(folder, { recursive: true, force: true })
  }

  return { socket, requests, close }
}

export const NUDGE = {
  rule: "conventional-commits",
  action: "message.append",
  payload: {
    label: "commit message isn't conventional",
    parts: [{ text: "Amend it to `type(scope): description`." }],
    delivery: "now"
  }
}
