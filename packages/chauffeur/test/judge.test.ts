import { expect, test } from "bun:test"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { connect } from "../src/client/index.ts"
import { ask, judge, noul } from "../src/judge/index.ts"
import { standIn } from "./stand-in.ts"

const ANSWER = {
  outcome: "yes" as const,
  p: { yes: 0.9, no: 0.1 },
  level: 0.1,
  model: "jev",
  placeholder: false
}

test("ask sends a one-step inline strategy naming the evidence it sends", async () => {
  const daemon = await standIn(() => ({
    result: { verdict: { ask: { answer: ANSWER } } }
  }))
  const chauffeur = connect({ socket: daemon.socket })
  const quick = judge({ tier: "system-one", confidence: 0.7 })

  try {
    const done = await ask(noul("Is the migration finished?"), {
      judge: quick,
      evidence: { log: "42 of 42" },
      deadlineMs: 1500,
      client: chauffeur
    })

    expect(done).toEqual({ ok: true, ...ANSWER })
    expect(daemon.messages[0]?.params).toEqual({
      protocol: 1,
      judges: { asked: { tier: "system-one", confidence: 0.7 } },
      strategy: [
        {
          id: "ask",
          judge: "asked",
          ask: {
            instructions: "Is the migration finished?",
            outcomes: ["yes", "no"],
            evidence: ["log"]
          }
        }
      ],
      evidence: { log: "42 of 42" },
      deadline_ms: 1500
    })
  } finally {
    chauffeur.close()
    await daemon.close()
  }
})

test("a failure, a rejection, no daemon and an abort are each ok: false with why", async () => {
  const failed = await standIn(() => ({
    result: { verdict: { ask: { failed: "timeout" } } }
  }))
  const busy = await standIn(() => ({
    error: { code: -32000, message: "busy", data: { reason: "busy" } }
  }))
  const quick = judge({ tier: "system-one" })
  const missing = join(tmpdir(), "chauffeur-missing", "daemon.sock")
  const aborted = AbortSignal.abort()

  try {
    const at = (socket: string) => ({ judge: quick, client: connect({ socket }) })

    expect(await ask(noul("?"), at(failed.socket))).toEqual({
      ok: false,
      reason: "timeout"
    })
    expect(await ask(noul("?"), at(busy.socket))).toEqual({
      ok: false,
      reason: "busy"
    })
    expect(await ask(noul("?"), at(missing))).toEqual({
      ok: false,
      reason: "unavailable"
    })
    expect(await ask(noul("?"), { ...at(failed.socket), signal: aborted })).toEqual({
      ok: false,
      reason: "cancelled"
    })
  } finally {
    await failed.close()
    await busy.close()
  }
})

test("a confidence outside 0 to 1 is refused before anything is sent", () => {
  expect(() => judge({ tier: "system-one", confidence: 1.5 })).toThrow(RangeError)
  expect(() =>
    judge({ tier: "system-one", confidence: { yes: 0.9, no: -1 } })
  ).toThrow(RangeError)
})
