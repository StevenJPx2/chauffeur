import { expect, test } from "bun:test"
import { DateTime, Effect } from "effect"
import type { DaemonClient } from "../src/daemon.js"
import type { Signal } from "../src/protocol.js"
import { SHIPPED_TEXTS } from "../src/texts.js"
import { installWarming, pricesAt } from "../src/warming.js"
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

function setup(session: { readonly parentID?: string }, warm: { readonly every_seconds: number; readonly times: number }) {
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

        return { text: "OK" }
      }),
    },
    model: { list: () => Effect.succeed({ data: [{ ...opus, cost }] }) },
  })

  const daemon: DaemonClient = { skills: noSkills, rulebooks: noRulebooks, texts: shippedTexts, signal: (value) => Effect.sync(() => {
    signals.push(value)

    return [{ type: "warm", agent_id: value.agent_id, ...warm }]
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

test("prices come from the tier the prompt reaches, in millionths of a dollar", () => {
  expect(pricesAt(cost, 120_000)).toEqual({ input: 4_000_000, cache_read: 200_000, cache_write: 5_000_000 })
  expect(pricesAt(cost, 250_000)).toEqual({ input: 8_000_000, cache_read: 400_000, cache_write: 10_000_000 })
  expect(pricesAt([], 120_000)).toEqual({ input: 0, cache_read: 0, cache_write: 0 })
})
