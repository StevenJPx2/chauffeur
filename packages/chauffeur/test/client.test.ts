import { expect, test } from "bun:test"
import { mkdtempSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { connect, navigate, NavigateError, socketPath } from "../src/client/index.ts"
import { eventually, standIn } from "./stand-in.ts"

const EVENT = { on: "sourcefed.event", evidence: { event: "thanks, looks good" } }
const WITHHOLD = { rule: "sourcefed/triage", withhold: "event" }

test("the socket is the option's, a name's, then $CHAUFFEUR_SOCKET, then the runtime folder's", () => {
  const env = { CHAUFFEUR_SOCKET: "/env/s", XDG_RUNTIME_DIR: "/run/user/1" }

  expect(socketPath({ socket: "/option/s" }, env)).toBe("/option/s")
  expect(socketPath({ name: "chauffeur-next" }, env)).toBe(
    "/run/user/1/chauffeur-next/daemon.sock"
  )
  expect(socketPath({}, env)).toBe("/env/s")
  expect(socketPath({}, { CHAUFFEUR_SOCKET: "" })).toBe(
    join(tmpdir(), "chauffeur/daemon.sock")
  )
})

test("navigate sends protocol 1 and the request, streams events, and closes once answered", async () => {
  const daemon = await standIn(() => ({
    events: [
      { event: "rule_matched", rule: "sourcefed/triage" },
      { event: "round_ready", rule: "sourcefed/triage", round: 1, steps: ["screen"] }
    ],
    result: { intents: [WITHHOLD] }
  }))
  const events: unknown[] = []

  try {
    const { intents } = await navigate(EVENT, {
      socket: daemon.socket,
      onEvent: (event) => events.push(event)
    })

    expect(intents).toEqual([WITHHOLD])
    expect(events).toEqual([
      { event: "rule_matched", rule: "sourcefed/triage" },
      { event: "round_ready", rule: "sourcefed/triage", round: 1, steps: ["screen"] }
    ])
    expect(daemon.messages[0]).toEqual({
      jsonrpc: "2.0",
      id: 1,
      method: "chauffeur.navigate",
      params: { protocol: 1, ...EVENT }
    } as never)
    await eventually(() => daemon.open === 0)
  } finally {
    await daemon.close()
  }
})

test("a rejection keeps its reason and limit; another protocol is unavailable", async () => {
  const daemon = await standIn((message) =>
    message.params.on === "old"
      ? {
          error: {
            code: -32602,
            message: "the daemon accepts protocol 2",
            data: { reason: "unsupported_protocol", accepted: [2] }
          }
        }
      : {
          error: {
            code: -32602,
            message: "evidence is over 256 KiB",
            data: { reason: "limit_exceeded", limit: "evidence_bytes", max: 262144 }
          }
        }
  )

  try {
    const over = navigate(EVENT, { socket: daemon.socket })
    await expect(over).rejects.toBeInstanceOf(NavigateError)
    await expect(over).rejects.toMatchObject({
      reason: "limit_exceeded",
      limit: "evidence_bytes",
      max: 262144
    })
    await expect(
      navigate({ on: "old" }, { socket: daemon.socket })
    ).rejects.toMatchObject({ reason: "unavailable" })
  } finally {
    await daemon.close()
  }
})

test("no daemon, one that hangs up, and one that never answers are all unavailable", async () => {
  const missing = join(tmpdir(), "chauffeur-missing", "daemon.sock")
  const hanging = await standIn(() => "hang-up")
  const silent = await standIn(() => "silent")

  try {
    await expect(navigate(EVENT, { socket: missing })).rejects.toMatchObject({
      reason: "unavailable"
    })
    await expect(navigate(EVENT, { socket: hanging.socket })).rejects.toMatchObject({
      reason: "unavailable"
    })

    const started = Date.now()
    await expect(
      navigate({ ...EVENT, deadline_ms: 10 }, { socket: silent.socket })
    ).rejects.toThrow("no answer within 2010 ms")
    expect(Date.now() - started).toBeLessThan(2_500)
  } finally {
    await hanging.close()
    await silent.close()
  }
})

test("aborting a one-shot navigate closes its connection, which cancels it", async () => {
  const daemon = await standIn(() => "silent")
  const abort = new AbortController()

  try {
    const pending = navigate(EVENT, { socket: daemon.socket, signal: abort.signal })
    await eventually(() => daemon.messages.length === 1)
    abort.abort(new Error("the caller gave up"))

    await expect(pending).rejects.toThrow("the caller gave up")
    await eventually(() => daemon.open === 0)
  } finally {
    await daemon.close()
  }
})

test("a kept connection carries calls at once, cancels an aborted one by ID, and closes", async () => {
  const daemon = await standIn((message) =>
    message.params.on === "slow" ? "silent" : { result: { intents: [] } }
  )
  const chauffeur = connect({ socket: daemon.socket })
  const abort = new AbortController()

  try {
    const slow = chauffeur.navigate({ on: "slow" }, { signal: abort.signal })
    const fast = await chauffeur.navigate(EVENT)
    abort.abort(new Error("timed out"))

    expect(fast).toEqual({ intents: [] })
    await expect(slow).rejects.toThrow("timed out")
    await eventually(() =>
      daemon.messages.some((message) => message.method === "$/cancelRequest")
    )
    const cancel = daemon.messages.find(
      (message) => message.method === "$/cancelRequest"
    )
    expect(cancel?.params).toEqual({ id: 1 })
    expect(daemon.closes).toBe(0)

    chauffeur.close()
    await eventually(() => daemon.open === 0)
  } finally {
    await daemon.close()
  }
})

test("a kept connection opens again after the daemon restarts", async () => {
  const socket = join(
    mkdtempSync(join(tmpdir(), "chauffeur-restart-")),
    "daemon.sock"
  )
  const first = await standIn(() => ({ result: { intents: [] } }), socket)
  const chauffeur = connect({ socket })

  try {
    expect(await chauffeur.navigate(EVENT)).toEqual({ intents: [] })
    await first.close()
    const second = await standIn(() => ({ result: { intents: [WITHHOLD] } }), socket)

    try {
      // A call racing the hang-up may still find the old connection closing.
      const raced = await chauffeur.navigate(EVENT).catch((error: unknown) => error)
      const answered =
        raced instanceof NavigateError ? await chauffeur.navigate(EVENT) : raced

      expect(answered).toEqual({ intents: [WITHHOLD] })
    } finally {
      await second.close()
    }
  } finally {
    chauffeur.close()
  }
})

test("a listener that throws ends its own call, and the others on the connection go on", async () => {
  const daemon = await standIn((message) => ({
    events: [{ event: "rule_matched", rule: String(message.params.on) }],
    result: { intents: [] }
  }))
  const chauffeur = connect({ socket: daemon.socket })

  try {
    const failing = chauffeur.navigate(
      { on: "a" },
      {
        onEvent: () => {
          throw new Error("the listener broke")
        }
      }
    )
    const fine = chauffeur.navigate({ on: "b" })

    await expect(failing).rejects.toThrow("the listener broke")
    expect(await fine).toEqual({ intents: [] })
  } finally {
    chauffeur.close()
    await daemon.close()
  }
})

test("a reason this client does not know reads as invalid", async () => {
  const daemon = await standIn(() => ({
    error: { code: -32000, message: "new", data: { reason: "something_new" } }
  }))

  try {
    await expect(navigate(EVENT, { socket: daemon.socket })).rejects.toMatchObject({
      reason: "invalid"
    })
  } finally {
    await daemon.close()
  }
})

test("a call on a closed client is unavailable, and opens nothing", async () => {
  const daemon = await standIn(() => ({ result: { intents: [] } }))
  const chauffeur = connect({ socket: daemon.socket })

  try {
    const racing = chauffeur.navigate(EVENT)
    chauffeur.close()

    await expect(racing).rejects.toMatchObject({ reason: "unavailable" })
    await expect(chauffeur.navigate(EVENT)).rejects.toMatchObject({
      reason: "unavailable"
    })
    await eventually(() => daemon.open === 0)
  } finally {
    await daemon.close()
  }
})
