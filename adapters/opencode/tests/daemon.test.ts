import { afterEach, expect, test } from "bun:test"
import { Effect, Exit } from "effect"
import { Daemon, type DaemonError } from "../src/daemon.js"
import { type HostEffect, signal } from "../src/protocol.js"

type WireEffect = { readonly [field: string]: string | boolean }

type Reply = { readonly result?: { readonly effects: ReadonlyArray<WireEffect> }; readonly error?: string }

const servers: Array<{ stop: () => void }> = []

const configured = process.env.CHAUFFEUR_DAEMON_URL

afterEach(() => {
  for (const server of servers.splice(0)) server.stop()

  if (configured === undefined) delete process.env.CHAUFFEUR_DAEMON_URL
  else process.env.CHAUFFEUR_DAEMON_URL = configured
})

/** A daemon that answers health checks and replies to signals with `reply`. */
function daemon(reply: Reply): void {
  const server = Bun.serve({
    port: 0,
    fetch: async (request) => {
      const { method } = await request.json()

      return Response.json(method === "health" ? { result: { ok: true } } : reply)
    },
  })

  servers.push(server)
  process.env.CHAUFFEUR_DAEMON_URL = `http://127.0.0.1:${server.port}`
}

const turnEnd = signal("ses_daemon", { type: "turn_end", workspace: "", user_request: "" })

async function send(): Promise<Exit.Exit<ReadonlyArray<HostEffect>, DaemonError>> {
  const client = await Effect.runPromise(Daemon.connect)

  return Effect.runPromiseExit(client.signal(turnEnd))
}

test("valid effects are decoded", async () => {
  daemon({ result: { effects: [{ type: "gate", agent_id: "ses_daemon", deliver: false }] } })

  expect(await send()).toEqual(Exit.succeed([{ type: "gate", agent_id: "ses_daemon", deliver: false }]))
})

test("an engine error or an invalid effect fails the signal", async () => {
  daemon({ error: "judge unavailable" })
  expect(Exit.isFailure(await send())).toBe(true)

  daemon({ result: { effects: [{ type: "permission" }] } })
  expect(Exit.isFailure(await send())).toBe(true)
})
