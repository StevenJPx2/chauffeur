import { expect, test } from "bun:test"
import { DateTime, Effect } from "effect"
import type { DaemonClient } from "../src/daemon.js"
import type { Signal } from "../src/protocol.js"
import { SHIPPED_TEXTS } from "../src/texts.js"
import { installWarming, missedCache, pricesAt } from "../src/warming.js"
import { eventStream, fakeHost, Hooks, install, noRulebooks, noSkills, shippedTexts } from "./support.js"

const opus = { providerID: "anthropic", id: "claude-opus-5-5" }

const cost = [
  { input: 4, output: 20, cache: { read: 0.2, write: 5 } },
  { tier: { type: "context", size: 200_000 }, input: 8, output: 30, cache: { read: 0.4, write: 10 } },
]

const reply = {
  type: "assistant",
  model: opus,
  tokens: { input: 1_000, output: 500, reasoning: 0, cache: { read: 118_500, write: 0 } },
  time: { created: DateTime.makeUnsafe(Date.now()), completed: DateTime.makeUnsafe(Date.now()) },
  content: [],
}

/** Wait for `ms` of real time, letting the warming loop run. */
function wait(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

function setup(session: { readonly parentID?: string }, warm: { readonly every_seconds: number; readonly times: number }, usage?: { readonly read: number; readonly write: number }, renewAfter?: number) {
  const events = eventStream()
  const hooks = new Hooks()
  const signals: Signal[] = []
  const generated: Array<{ readonly sessionID: string; readonly prompt: string }> = []

  const host = fakeHost({
    location: { directory: "/projects/app" },
    event: { subscribe: events.subscribe },
    session: {
      hook: hooks.register,
      get: () => Effect.succeed({ location: { directory: "/projects/app" }, model: opus, ...session }),
      context: () => Effect.succeed([reply]),
      generate: (input: (typeof generated)[number]) => Effect.sync(() => {
        generated.push(input)

        return usage ? { text: "OK", tokens: { input: 4, output: 4, reasoning: 0, cache: usage } } : { text: "OK" }
      }),
    },
    model: { list: () => Effect.succeed({ data: [{ ...opus, cost, limit: { context: 1_000_000 } }] }) },
  })

  const daemon: DaemonClient = { skills: noSkills, rulebooks: noRulebooks, texts: shippedTexts, signal: (value) => Effect.sync(() => {
    signals.push(value)

    return [
      { type: "warm", agent_id: value.agent_id, ...warm },
      ...(renewAfter === undefined ? [] : [{ type: "renew_when_idle" as const, agent_id: value.agent_id, after_seconds: renewAfter }]),
    ]
  }) }

  return { events, hooks, signals, generated, host, daemon }
}

test("a finished turn reports its context and prices, and the engine's warming runs until the session's next request", async () => {
  const { events, hooks, signals, generated, host, daemon } = setup({}, { every_seconds: 0.02, times: 50 })
  const plugin = await install(installWarming, host, daemon)

  try {
    await events.publish({ type: "session.execution.succeeded", data: { sessionID: "ses_idle" } })
    await wait(90)

    expect(signals[0]?.kind).toEqual({
      type: "context_idle",
      model: { provider: "anthropic", model: "claude-opus-5-5", variant: undefined },
      context_tokens: 120_000,
      context_window: 1_000_000,
      awaiting_answer: false,
      prices: { input: 4_000_000, cache_read: 200_000, cache_write: 5_000_000 },
    })
    expect(generated.length).toBeGreaterThanOrEqual(2)
    expect(generated[0]).toEqual({ sessionID: "ses_idle", prompt: SHIPPED_TEXTS.warming.prompt })
    expect(plugin.value.lastWarmed("ses_idle")).toBeGreaterThan(Date.now() - 1_000)

    // The session's own request ends the warming.
    await hooks.emit("context", { sessionID: "ses_idle" })
    await wait(10)

    const sent = generated.length

    await wait(80)
    expect(generated).toHaveLength(sent)
  } finally {
    await plugin.close()
  }
})

test("warming stops after the engine's count, and a subagent's session is never warmed", async () => {
  const counted = setup({}, { every_seconds: 0.01, times: 2 })
  const subagent = setup({ parentID: "ses_parent" }, { every_seconds: 0.01, times: 2 })
  const first = await install(installWarming, counted.host, counted.daemon)
  const second = await install(installWarming, subagent.host, subagent.daemon)

  try {
    await counted.events.publish({ type: "session.execution.succeeded", data: { sessionID: "ses_main" } })
    await subagent.events.publish({ type: "session.execution.succeeded", data: { sessionID: "ses_child" } })
    await wait(100)

    expect(counted.generated).toHaveLength(2)
    expect(subagent.signals).toEqual([])
    expect(subagent.generated).toEqual([])
  } finally {
    await first.close()
    await second.close()
  }
})

test("a session the engine said to renew once idle is handed to the expiry listener with its delay", async () => {
  const { events, generated, host, daemon } = setup({}, { every_seconds: 60, times: 1 }, undefined, 3_660)
  const plugin = await install(installWarming, host, daemon)
  const told: Array<readonly [string, number]> = []

  plugin.value.expiring((sessionID, afterSeconds) => Effect.sync(() => { told.push([String(sessionID), afterSeconds]) }))

  try {
    await events.publish({ type: "session.execution.succeeded", data: { sessionID: "ses_long" } })
    await wait(30)

    expect(told).toEqual([["ses_long", 3_660]])
    expect(generated).toEqual([])
  } finally {
    await plugin.close()
  }
})

test("a question the agent asks the user is reported as awaiting an answer, never warmed, and handed to the expiry listener", async () => {
  const { events, signals, generated, host, daemon } = setup({}, { every_seconds: 0.02, times: 2 }, undefined, 3_660)
  const plugin = await install(installWarming, host, daemon)
  const told: Array<readonly [string, number, unknown]> = []

  const form = (id: string, kind: string) => ({ type: "form.created", data: { form: { id, sessionID: "ses_asking", title: "Questions", metadata: { kind }, fields: [
    { key: "q0", title: "Colour", description: "Red or blue?", type: "string", options: [{ value: "Red", label: "Red", description: "Choose red" }], custom: true },
  ] } } })

  plugin.value.expiring((sessionID, afterSeconds, question) => Effect.sync(() => { told.push([String(sessionID), afterSeconds, question]) }))

  try {
    await events.publish(form("frm_mcp", "elicitation"))
    await events.publish(form("frm_question", "question"))
    await wait(80)

    expect(signals).toHaveLength(1)
    expect(signals[0]?.kind).toMatchObject({ type: "context_idle", awaiting_answer: true, context_tokens: 120_000 })
    expect(told).toEqual([["ses_asking", 3_660, {
      id: "frm_question",
      asked: expect.any(Number),
      questions: [{ question: "Red or blue?", header: "Colour", options: [{ label: "Red", description: "Choose red" }], multiple: false }],
    }]])
    // Its context ends in the unanswered call: the engine's warming is never sent.
    expect(generated).toEqual([])
  } finally {
    await plugin.close()
  }
})

test("a warming request that misses the cache moves the session's renewal up to its cache lifetime after the turn", async () => {
  // Warmed every 0.05 s, 3 times; renewal planned 0.15 s of warming plus 0.1 s of lifetime and margin.
  const { events, host, daemon } = setup({}, { every_seconds: 0.05, times: 3 }, { read: 1_217, write: 118_000 }, 0.25)
  const plugin = await install(installWarming, host, daemon)
  const told: Array<readonly [string, number]> = []

  plugin.value.expiring((sessionID, afterSeconds) => Effect.sync(() => { told.push([String(sessionID), afterSeconds]) }))

  try {
    await events.publish({ type: "session.execution.succeeded", data: { sessionID: "ses_missed" } })
    await wait(120)

    expect(told[0]).toEqual(["ses_missed", 0.25])
    // The first warming request missed at about 0.05 s: renewal 0.1 s after the turn, about 0.05 s on.
    expect(told).toHaveLength(2)
    expect(told[1]?.[1]).toBeLessThan(0.1)
  } finally {
    await plugin.close()
  }
})

test("a warming request that rebuilt the cache instead of reading it stops the warming and does not count as warm", async () => {
  const missed = setup({}, { every_seconds: 0.01, times: 5 }, { read: 1_217, write: 118_000 })
  const kept = setup({}, { every_seconds: 0.01, times: 3 }, { read: 118_000, write: 44 })
  const first = await install(installWarming, missed.host, missed.daemon)
  const second = await install(installWarming, kept.host, kept.daemon)

  try {
    await missed.events.publish({ type: "session.execution.succeeded", data: { sessionID: "ses_miss" } })
    await kept.events.publish({ type: "session.execution.succeeded", data: { sessionID: "ses_hit" } })
    await wait(100)

    expect(missed.generated).toHaveLength(1)
    expect(first.value.lastWarmed("ses_miss")).toBeUndefined()
    expect(kept.generated).toHaveLength(3)
    expect(second.value.lastWarmed("ses_hit")).toBeGreaterThan(Date.now() - 1_000)
  } finally {
    await first.close()
    await second.close()
  }
})

test("a reply misses the cache when it wrote more than it read; one without usage never does", () => {
  const reply = (read: number, write: number) => ({ text: "OK", tokens: { cache: { read, write } } })

  expect(missedCache(reply(1_217, 447_352))).toBe(true)
  expect(missedCache(reply(13_007, 44))).toBe(false)
  expect(missedCache({ text: "OK" })).toBe(false)
})

test("prices come from the tier the prompt reaches, in millionths of a dollar", () => {
  expect(pricesAt(cost, 120_000)).toEqual({ input: 4_000_000, cache_read: 200_000, cache_write: 5_000_000 })
  expect(pricesAt(cost, 250_000)).toEqual({ input: 8_000_000, cache_read: 400_000, cache_write: 10_000_000 })
  expect(pricesAt([], 120_000)).toEqual({ input: 0, cache_read: 0, cache_write: 0 })
})
