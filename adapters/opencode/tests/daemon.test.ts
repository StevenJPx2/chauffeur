import { afterEach, expect, test } from "bun:test"
import { Effect, Exit, Schema } from "effect"
import { Daemon, type DaemonError } from "../src/daemon.js"
import { type HostEffect, signal } from "../src/protocol.js"

type WireEffect = { readonly [field: string]: string | boolean }

type Reply = { readonly result?: { readonly effects: ReadonlyArray<WireEffect> }; readonly error?: string }

const SignalBudget = Schema.Struct({ deadline_ms: Schema.optional(Schema.Number) })

type SignalBudget = typeof SignalBudget.Type

const servers: Array<{ stop: () => void }> = []

const configured = process.env.CHAUFFEUR_DAEMON_URL

afterEach(() => {
  for (const server of servers.splice(0)) server.stop()

  if (configured === undefined) delete process.env.CHAUFFEUR_DAEMON_URL
  else process.env.CHAUFFEUR_DAEMON_URL = configured
})

/** A daemon that answers health checks and replies to signals with `reply`. */
function daemon(reply: Reply | Response, observe: (budget: SignalBudget) => void = () => undefined): void {
  const server = Bun.serve({
    port: 0,
    fetch: async (request) => {
      const wire = await request.json()
      const { method } = wire

      if (method === "health") return Response.json({ result: { ok: true } })

      observe(Schema.decodeUnknownSync(SignalBudget)(wire.params))

      return reply instanceof Response ? reply : Response.json(reply)
    },
  })

  servers.push(server)
  process.env.CHAUFFEUR_DAEMON_URL = `http://127.0.0.1:${server.port}`
}

const turnEnd = signal("ses_daemon", { type: "turn_end", workspace: "", subagent: false, user_request: "", summary: "", todos: [] })

async function send(): Promise<Exit.Exit<ReadonlyArray<HostEffect>, DaemonError>> {
  const client = await Effect.runPromise(Daemon.connect)

  return Effect.runPromiseExit(client.signal(turnEnd))
}

test("valid effects are decoded", async () => {
  daemon({ result: { effects: [{ type: "gate", agent_id: "ses_daemon", deliver: false }] } })

  expect(await send()).toEqual(Exit.succeed([{ type: "gate", agent_id: "ses_daemon", deliver: false }]))
})

test("signals carry each hook's timeout and the default budget", async () => {
  const seen: SignalBudget[] = []
  daemon({ result: { effects: [] } }, (budget) => seen.push(budget))
  const client = await Effect.runPromise(Daemon.connect)

  for (const seconds of [2, 6, 3, 8]) await Effect.runPromise(client.signal(turnEnd, `${seconds} seconds`))

  await Effect.runPromise(client.signal(turnEnd))

  expect(seen).toEqual([2_000, 6_000, 3_000, 8_000, 20_000].map((deadline_ms) => ({ deadline_ms })))
})

test("an older strict daemon receives a signal without the deadline field", async () => {
  const seen: unknown[] = []

  const server = Bun.serve({
    port: 0,
    fetch: async (request) => {
      const wire = await request.json()

      if (wire.method === "health") return Response.json({ result: { ok: true } })

      seen.push(wire.params)

      return Response.json(wire.params.deadline_ms === undefined
        ? { result: { effects: [] } }
        : { error: "invalid params: unknown field `deadline_ms`, expected `signal`" })
    },
  })

  servers.push(server)
  process.env.CHAUFFEUR_DAEMON_URL = `http://127.0.0.1:${server.port}`

  expect(await send()).toEqual(Exit.succeed([]))
  expect(seen).toEqual([{ signal: turnEnd, deadline_ms: 20_000 }, { signal: turnEnd }])
})

test("an engine error or an invalid effect fails the signal", async () => {
  daemon({ error: "judge unavailable" })
  expect(Exit.isFailure(await send())).toBe(true)

  daemon({ result: { effects: [{ type: "permission" }] } })
  expect(Exit.isFailure(await send())).toBe(true)
})

test("a non-JSON error reply reports its HTTP status", async () => {
  daemon(new Response("Bad Gateway", { status: 502 }))

  const exit = await send()

  expect(Exit.isFailure(exit) && String(exit.cause)).toContain("chauffeur daemon returned 502")
})
