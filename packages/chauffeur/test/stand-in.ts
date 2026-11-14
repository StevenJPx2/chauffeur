import { mkdtempSync, rmSync } from "node:fs"
import { createServer, type Socket } from "node:net"
import { tmpdir } from "node:os"
import { join } from "node:path"

export type Message = {
  readonly id?: number
  readonly method: string
  readonly params: Record<string, unknown>
}

/** What a stand-in does with one request: reply, stream events first, or stay silent. */
export type Reply =
  | {
      readonly result: unknown
      readonly events?: readonly Record<string, unknown>[]
    }
  | { readonly error: Record<string, unknown> }
  | "silent"
  | "hang-up"

/** A daemon on a socket in a fresh folder, answering each request with `reply`. */
export async function standIn(reply: (message: Message) => Reply, path?: string) {
  const folder = path ? undefined : mkdtempSync(join(tmpdir(), "chauffeur-client-"))
  const socket = path ?? join(folder ?? "", "daemon.sock")
  const messages: Message[] = []
  const connections = new Set<Socket>()
  let closes = 0

  const server = createServer((connection) => {
    connections.add(connection)
    connection.setEncoding("utf8")
    let received = ""
    connection.on("close", () => {
      connections.delete(connection)
      closes += 1
    })
    connection.on("data", (data: string) => {
      received += data
      let end = received.indexOf("\n")

      while (end !== -1) {
        const message = JSON.parse(received.slice(0, end)) as Message
        received = received.slice(end + 1)
        messages.push(message)
        answer(connection, message)
        end = received.indexOf("\n")
      }
    })
  })

  const answer = (connection: Socket, message: Message) => {
    if (message.id === undefined) return

    const replied = reply(message)

    if (replied === "silent") return

    if (replied === "hang-up") {
      connection.destroy()
      return
    }

    const send = (body: unknown) => connection.write(`${JSON.stringify(body)}\n`)

    if ("result" in replied) {
      for (const event of replied.events ?? []) {
        send({
          jsonrpc: "2.0",
          method: "chauffeur.event",
          params: { ...event, id: message.id }
        })
      }
      send({ jsonrpc: "2.0", id: message.id, result: replied.result })
    } else {
      send({ jsonrpc: "2.0", id: message.id, error: replied.error })
    }
  }

  await new Promise<void>((resolve) => server.listen(socket, resolve))

  return {
    socket,
    messages,
    get open() {
      return connections.size
    },
    get closes() {
      return closes
    },
    async close() {
      for (const connection of connections) connection.destroy()
      await new Promise<void>((resolve) => server.close(() => resolve()))
      if (folder) rmSync(folder, { recursive: true, force: true })
    }
  }
}

/** Waits until `done` holds, at most two seconds. */
export async function eventually(done: () => boolean): Promise<void> {
  const started = Date.now()

  while (!done()) {
    if (Date.now() - started > 2_000) throw new Error("it never happened")
    await new Promise((resolve) => setTimeout(resolve, 5))
  }
}
