import { expect, test } from "bun:test"
import { Data, DateTime, Effect } from "effect"
import type { DaemonClient } from "../src/daemon.js"
import type { HistoryMessage } from "../src/host.js"
import type { Signal } from "../src/protocol.js"
import { contextFacts, installRenewal, summaryRequest } from "../src/renewal.js"
import { SHIPPED_TEXTS } from "../src/texts.js"
import { fakeHost, type FakeHost, Hooks, install, noRulebooks, noSkills, sessionID as session, shippedTexts } from "./support.js"

const opus = { providerID: "anthropic", id: "claude-opus-5-5", variant: "high" }

const models = { data: [
  { ...opus, enabled: true, status: "active", limit: { context: 1_000_000 } },
  { providerID: "anthropic", id: "claude-haiku-5", enabled: true, status: "active", limit: { context: 200_000 } },
] }

const summary = "## Objective\n- Ship the feature"

/** OpenCode's compaction hook event, with the fields the renewal reads and writes. */
type CompactionEvent = { readonly sessionID: string; readonly messages: ReadonlyArray<unknown>; result?: { readonly summary: string } }

function reply(minutesAgo: number, tokens: number) {
  const at = DateTime.makeUnsafe(Date.now() - minutesAgo * 60_000)

  return {
    type: "assistant",
    model: opus,
    tokens: { input: tokens, output: 0, reasoning: 0, cache: { read: 0, write: 0 } },
    time: { created: at, completed: at },
    content: [],
  }
}

function setup(history: ReadonlyArray<unknown>, compact: boolean, overrides: FakeHost = {}) {
  const hooks = new Hooks()
  const signals: Signal[] = []
  const compactions: string[] = []
  const generated: Array<{ readonly prompt: string; readonly model: { readonly id: string } }> = []

  const host = fakeHost({
    session: {
      hook: hooks.register,
      context: () => Effect.succeed(history),
      get: () => Effect.succeed({ model: opus }),
      compact: (input: { readonly sessionID: string }) => Effect.sync(() => { compactions.push(input.sessionID) }),
    },
    model: { list: () => Effect.succeed(models) },
    storage: { get: () => Effect.succeed(undefined), set: () => Effect.void },
    generate: {
      text: (input: (typeof generated)[number]) => Effect.sync(() => {
        generated.push(input)

        return { text: input.model.id === "claude-haiku-5" ? summary : "no template here" }
      }),
    },
    ...overrides,
  })

  const daemon: DaemonClient = { skills: noSkills, rulebooks: noRulebooks, texts: shippedTexts, signal: (value) => Effect.sync(() => {
    signals.push(value)

    return compact ? [{ type: "compact", agent_id: value.agent_id, model: { provider: "anthropic", model: "claude-haiku-5" } }] : []
  }) }

  return { hooks, signals, compactions, generated, host, daemon }
}

test("a cold context reports its size and idle time; the engine's compaction is queued and summarized by the cheaper model", async () => {
  const { hooks, signals, compactions, generated, host, daemon } = setup([reply(30, 640_000)], true)
  const plugin = await install(installRenewal(), host, daemon)

  try {
    expect(await Effect.runPromise(plugin.value.renew(session("ses_cold")))).toBe(true)
    expect(signals[0]?.kind).toMatchObject({
      type: "context_resumed",
      model: { provider: "anthropic", model: "claude-opus-5-5", variant: "high" },
      context_tokens: 640_000,
      context_window: 1_000_000,
    })
    expect(signals[0]?.kind.type === "context_resumed" && signals[0].kind.idle_seconds).toBeGreaterThanOrEqual(30 * 60)
    expect(compactions).toEqual(["ses_cold"])

    const event: CompactionEvent = {
      sessionID: "ses_cold",
      messages: [{ role: "user", content: [{ type: "text", text: "build the feature" }] }],
    }

    await hooks.emit("compaction", event)

    expect(generated[0]?.model.id).toBe("claude-haiku-5")
    expect(generated[0]?.prompt).toContain("[User]: build the feature")
    expect(event.result?.summary).toBe(summary)

    // Only the compaction Chauffeur queued is summarized by the cheaper model.
    const auto: CompactionEvent = { sessionID: "ses_cold", messages: [] }

    await hooks.emit("compaction", auto)
    expect(auto.result).toBeUndefined()
    expect(generated).toHaveLength(1)
  } finally {
    await plugin.close()
  }
})

test("no compaction is queued when the engine declines, or the context has no measured reply", async () => {
  const declined = setup([reply(30, 640_000)], false)
  const fresh = setup([], true)
  const first = await install(installRenewal(), declined.host, declined.daemon)
  const second = await install(installRenewal(), fresh.host, fresh.daemon)

  try {
    expect(await Effect.runPromise(first.value.renew(session("ses_warm")))).toBe(false)
    expect(declined.compactions).toEqual([])
    expect(await Effect.runPromise(second.value.renew(session("ses_new")))).toBe(false)
    expect(fresh.signals).toEqual([])
  } finally {
    await first.close()
    await second.close()
  }
})

test("a summary that skips the template leaves the compaction to OpenCode's own model", async () => {
  const { hooks, host, daemon } = setup([reply(30, 640_000)], true)

  const plugin = await install(installRenewal(), host, {
    ...daemon,
    signal: (value) => Effect.succeed([{ type: "compact", agent_id: value.agent_id, model: { provider: "anthropic", model: "claude-opus-5-5" } }]),
  })

  try {
    await Effect.runPromise(plugin.value.renew(session("ses_bad")))

    const event: CompactionEvent = { sessionID: "ses_bad", messages: [] }

    await hooks.emit("compaction", event)
    expect(event.result).toBeUndefined()
  } finally {
    await plugin.close()
  }
})

/** The host's error for a provider that cannot serve a plain generation, as OpenCode raises it. */
class Unavailable extends Data.TaggedError("Generate.UnavailableError")<{ readonly message: string; readonly service: string }> {}

test("a provider that cannot serve the summary is passed over at the next renewal", async () => {
  const saved: Array<Readonly<Record<string, number>>> = []

  const { hooks, signals, host, daemon } = setup([reply(30, 640_000)], true, {
    storage: {
      get: () => Effect.succeed(undefined),
      set: (_key: string, deadlines: Readonly<Record<string, number>>) => Effect.sync(() => { saved.push(deadlines) }),
    },
    generate: { text: () => Effect.fail(new Unavailable({ message: "Error", service: "anthropic" })) },
  })

  const plugin = await install(installRenewal(), host, daemon)

  try {
    await Effect.runPromise(plugin.value.renew(session("ses_oauth")))
    await hooks.emit("compaction", { sessionID: "ses_oauth", messages: [] })
    await Effect.runPromise(plugin.value.renew(session("ses_oauth")))

    const usable = (signal: Signal | undefined) =>
      signal?.kind.type === "context_resumed" ? signal.kind.available.map((entry) => entry.usable) : []

    expect(usable(signals[0])).toEqual([true, true])
    expect(usable(signals[1])).toEqual([false, false])
    expect(saved).toEqual([{ anthropic: expect.any(Number) }])
  } finally {
    await plugin.close()
  }
})

/** Session history with only the fields the facts read. */
function history(...entries: ReadonlyArray<unknown>): ReadonlyArray<HistoryMessage> {
  // SAFETY: contextFacts reads only type, error, tokens, time and model, which each entry carries.
  return entries as ReadonlyArray<HistoryMessage>
}

test("facts come from the latest measured reply since the last compaction", () => {
  const now = Date.now()

  expect(contextFacts(history(reply(10, 5_000), { type: "compaction" }), now)).toBeUndefined()
  expect(contextFacts(history(reply(10, 5_000), reply(2, 9_000)), now)).toMatchObject({ tokens: 9_000, idleSeconds: 120 })
})

test("a warming request after the latest reply restarts the idle time", async () => {
  const { signals, host, daemon } = setup([reply(50, 640_000)], false)
  const plugin = await install(installRenewal({ lastWarmed: () => Date.now() - 3 * 60_000 }), host, daemon)

  try {
    await Effect.runPromise(plugin.value.renew(session("ses_warmed")))

    const idle = signals[0]?.kind.type === "context_resumed" ? signals[0].kind.idle_seconds : undefined

    expect(idle).toBeGreaterThanOrEqual(180)
    expect(idle).toBeLessThan(240)
  } finally {
    await plugin.close()
  }
})

test("the transcript keeps an earlier summary and the newest exchanges that fit", () => {
  const messages = [
    { role: "user", content: [{ type: "text", text: "<conversation-checkpoint><summary>earlier</summary></conversation-checkpoint>" }] },
    { role: "user", content: [{ type: "text", text: "old ".repeat(400_000) }] },
    { role: "user", content: [{ type: "text", text: "newest request" }] },
    { role: "assistant", content: [{ type: "tool-result", name: "read", result: { type: "text", value: "x".repeat(5_000) } }] },
  ]

  // @ts-expect-error -- hook messages carry only the parts the transcript reads.
  const prompt = summaryRequest(messages, 200_000, SHIPPED_TEXTS)

  expect(prompt).toContain("earlier")
  expect(prompt).toContain("[1 older exchanges omitted]")
  expect(prompt).toContain("[User]: newest request")
  expect(prompt).toContain("[truncated]")
  expect(prompt).toContain(SHIPPED_TEXTS.compaction.update.slice(0, 40))
})
