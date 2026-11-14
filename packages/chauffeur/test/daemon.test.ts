// The bindings against the real daemon: the fixtures every binding replays,
// the same verdicts from the client, `/judge` and `chauffeur navigate`, and an
// abort from each cancelling the navigation. They start the daemon and stop
// it with a signal, so they run where CI sets CHAUFFEUR_BIN, never on a
// developer's machine.
import { afterAll, beforeAll, describe, expect, test } from "bun:test"
import { spawn, spawnSync, type ChildProcess } from "node:child_process"
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync
} from "node:fs"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import Ajv2020 from "ajv/dist/2020.js"
import {
  navigate,
  NavigateError,
  type NavigateRequest
} from "../src/client/index.ts"
import { ask, judge, noul, connect } from "../src/judge/index.ts"
import { eventually } from "./stand-in.ts"

const ROOT = resolve(import.meta.dir, "../../..")
const BIN = process.env.CHAUFFEUR_BIN

type Case = {
  readonly name: string
  readonly params: Record<string, unknown>
  readonly expect?: "on" | "strategy"
  readonly reject?: { readonly reason: string; readonly limit?: string }
  readonly fill?: readonly {
    in: string
    name: string
    text: string
    times: number
  }[]
}

/** The fixtures, each with its `fill` applied. */
function cases(): Case[] {
  const text = readFileSync(
    join(ROOT, "crates/protocol/fixtures/navigate.json"),
    "utf8"
  )
  const { cases } = JSON.parse(text) as { cases: Case[] }

  return cases.map((each) => {
    const params = structuredClone(each.params) as Record<
      string,
      Record<string, string>
    >

    for (const fill of each.fill ?? []) {
      params[fill.in] = {
        ...params[fill.in],
        [fill.name]: fill.text.repeat(fill.times)
      }
    }

    return { ...each, params }
  })
}

test("every accepted fixture fits the schema", () => {
  const schema = JSON.parse(
    readFileSync(join(ROOT, "crates/protocol/schema/navigate.json"), "utf8")
  )
  const validate = new Ajv2020({ strict: false }).compile(schema)

  for (const each of cases().filter((each) => each.expect)) {
    expect({ name: each.name, valid: validate(each.params) }).toEqual({
      name: each.name,
      valid: true
    })
  }
})

test("CI runs the tests against the real daemon", () => {
  if (process.env.CI) expect(BIN).toBeString()
})

describe.skipIf(!BIN)("against the real daemon", () => {
  const root = mkdtempSync(join(tmpdir(), "chauffeur-bindings-"))
  const socket = join(root, "run", "daemon.sock")
  const env = {
    ...process.env,
    CHAUFFEUR_SOCKET: socket,
    XDG_CONFIG_HOME: join(root, "config"),
    XDG_STATE_HOME: join(root, "state"),
    CHAUFFEUR_DISTRIBUTION: join(ROOT, "distribution")
  }
  let daemon: ChildProcess | undefined
  let jev: ReturnType<typeof Bun.serve> | undefined

  beforeAll(async () => {
    // A Jev that answers yes after three seconds, for the aborts.
    jev = Bun.serve({
      port: 0,
      hostname: "127.0.0.1",
      async fetch() {
        await Bun.sleep(3_000)
        return Response.json({ answers: { q0: { type: "noul", noul: 0.97 } } })
      }
    })
    const models = {
      models: {
        "stand-in": { format: "placeholder", otherwise: { yes: 0.1, no: 0.9 } },
        slow: {
          format: "systemone",
          endpoint: `http://127.0.0.1:${jev.port}`,
          model: "jev-latest",
          key: { env: "HOME" },
          timeout_ms: 5000
        }
      },
      tiers: { "system-one": ["stand-in"] }
    }
    mkdirSync(join(root, "config/chauffeur/rules/sourcefed"), { recursive: true })
    writeFileSync(join(root, "models.json"), JSON.stringify(models))
    writeFileSync(
      join(root, "config/chauffeur/rules/sourcefed/triage.json"),
      JSON.stringify({
        on: "sourcefed.event",
        strategy: [
          {
            id: "screen",
            judge: "system-one",
            ask: {
              outcomes: ["yes", "no"],
              evidence: ["event"],
              instructions: "Show it?"
            }
          }
        ],
        then: [{ if: { screen: "no" }, withhold: "event" }]
      })
    )
    daemon = spawn(
      BIN ?? "",
      ["daemon", "run", "--models", join(root, "models.json")],
      { env, stdio: "ignore" }
    )
    await eventually(() => existsSync(socket))
  })

  afterAll(() => {
    daemon?.kill()
    jev?.stop(true)
    rmSync(root, { recursive: true, force: true })
  })

  /** `chauffeur navigate` with `request`: its exit status and its last line. */
  const cli = (request: unknown) => {
    const ran = spawnSync(BIN ?? "", ["navigate"], {
      env,
      input: JSON.stringify(request),
      encoding: "utf8"
    })
    const last = ran.stdout.trim().split("\n").pop() ?? "{}"
    return { status: ran.status, reply: JSON.parse(last) as Record<string, any> }
  }

  test("the client and chauffeur navigate accept and reject every fixture alike", async () => {
    for (const each of cases()) {
      const client: unknown = await navigate(each.params as NavigateRequest, {
        socket
      }).then(
        (result) => ({ result }),
        (error: NavigateError) => ({ reason: error.reason, limit: error.limit })
      )
      const { status, reply } = cli(each.params)

      if (each.expect) {
        expect({ name: each.name, client }).toEqual({
          name: each.name,
          client: { result: reply.result }
        })
        expect({ name: each.name, status }).toEqual({ name: each.name, status: 0 })
      } else {
        const reason =
          each.reject?.reason === "unsupported_protocol"
            ? "unavailable"
            : each.reject?.reason
        const rejected: Record<string, unknown> = {
          reason,
          limit: each.reject?.limit
        }
        expect({ name: each.name, client }).toEqual({
          name: each.name,
          client: rejected
        })
        expect({ name: each.name, status }).toEqual({
          name: each.name,
          status: reason === "unavailable" ? 2 : 1
        })
      }
    }
  }, 60_000)

  test("ask gives the verdict the client and chauffeur navigate give", async () => {
    const evidence = { log: "migrated 42 of 42 tables" }
    const quick = judge({ tier: "system-one", confidence: 0.7 })
    const request = {
      judges: { asked: quick },
      strategy: [
        {
          id: "ask",
          judge: "asked",
          ask: { ...noul("Is the migration finished?"), evidence: ["log"] }
        }
      ],
      evidence
    }

    const done = await ask(noul("Is the migration finished?"), {
      judge: quick,
      evidence,
      client: connect({ socket })
    })
    const { verdict } = await navigate(request, { socket })
    const { reply } = cli({ protocol: 1, ...request })

    expect(verdict).toEqual(reply.result.verdict)
    expect(done).toEqual({
      ok: true,
      ...(verdict.ask as { answer: object }).answer
    } as never)
    expect(done.ok && done.outcome).toBe("no")
  })

  const slow = {
    judges: { careful: { tier: "system-one" as const, model: "slow" } },
    strategy: [
      {
        id: "done",
        judge: "careful",
        ask: { outcomes: ["yes", "no"], instructions: "Finished?" }
      }
    ],
    deadline_ms: 10_000
  }

  /** How many runs the audit log records as cancelled. */
  const cancelled = () => {
    const log = join(root, "state/chauffeur/audit.jsonl")
    const lines = existsSync(log) ? readFileSync(log, "utf8").trim().split("\n") : []
    return lines.filter((line) => line.includes('"cancelled":true')).length
  }

  test("aborting the client, ask or chauffeur navigate cancels the navigation", async () => {
    const before = cancelled()

    await expect(
      navigate(slow, { socket, signal: AbortSignal.timeout(200) })
    ).rejects.toThrow()
    await eventually(() => cancelled() === before + 1)

    const asked = await ask(
      { instructions: "Finished?", outcomes: ["yes", "no"] },
      {
        judge: judge({ tier: "system-one", model: "slow" }),
        client: connect({ socket }),
        deadlineMs: 10_000,
        signal: AbortSignal.timeout(200)
      }
    )
    expect(asked).toEqual({ ok: false, reason: "cancelled" })
    await eventually(() => cancelled() === before + 2)

    const child = spawn(BIN ?? "", ["navigate"], {
      env,
      stdio: ["pipe", "ignore", "ignore"]
    })
    child.stdin?.end(JSON.stringify({ protocol: 1, ...slow }))
    await Bun.sleep(300)
    child.kill()
    await eventually(() => cancelled() === before + 3)
  }, 15_000)
})
