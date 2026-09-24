import { expect, test } from "bun:test"
import { Deferred, Effect } from "effect"
import type { DaemonClient } from "../src/daemon.js"
import { installModelRouter } from "../src/model-router.js"
import type { HostEffect, Signal } from "../src/protocol.js"
import { eventStream, fakeHost, Hooks, install, settle } from "./support.js"

const failed = { providerID: "anthropic", id: "claude-opus-5-5", variant: "high" }

const models = { data: [
  { ...failed, enabled: true, status: "active" },
  { providerID: "openai", id: "gpt-6-sol", enabled: true, status: "active" },
] }

const retry = (sessionID: string) => ({
  sessionID,
  model: failed,
  error: { type: "provider.rate_limit", status: 429, message: "limited" },
  decision: { retry: true, delay: 1000 },
})

test("a tool in an earlier request does not block failover; a tool in this request does", async () => {
  const sessionID = "ses_test"
  const hooks = new Hooks()
  const events = eventStream()
  const signals: Signal[] = []
  const switches: string[] = []

  const host = fakeHost({
    session: {
      hook: hooks.register,
      switchModel: (input: { readonly model: { readonly id: string } }) => Effect.sync(() => { switches.push(input.model.id) }),
      get: () => Effect.succeed({ model: failed }),
    },
    model: { list: () => Effect.succeed(models) },
    tool: { hook: hooks.register },
    event: { subscribe: events.subscribe },
  })

  const daemon: DaemonClient = { signal: (value) => Effect.sync(() => {
    signals.push(value)

    return value.kind.type === "model_error" && !value.kind.tool_executed
      ? [{ type: "model", agent_id: sessionID, model: { provider: "openai", model: "gpt-6-sol" } }]
      : [{ type: "model", agent_id: sessionID, model: null }]
  }) }

  const plugin = await install(installModelRouter, host, daemon)

  try {
    await events.publish({ type: "session.execution.started", data: { sessionID } })
    await settle()
    await hooks.emit("model.request", { sessionID, kind: "primary", model: failed })
    await hooks.emit("execute.after", { sessionID })
    await hooks.emit("model.request", { sessionID, kind: "primary", model: failed })

    const safe = retry(sessionID)

    await hooks.emit("retry", safe)
    expect(signals.at(-1)?.kind).toMatchObject({ tool_executed: false })
    expect(switches).toEqual(["gpt-6-sol"])
    expect(safe.decision).toEqual({ retry: true, delay: 0 })

    await hooks.emit("model.request", { sessionID, kind: "primary", model: failed })
    await hooks.emit("execute.after", { sessionID })

    const unsafe = retry(sessionID)

    await hooks.emit("retry", unsafe)
    expect(signals.at(-1)?.kind).toMatchObject({ tool_executed: true })
    expect(switches).toHaveLength(1)
    expect(unsafe.decision).toEqual({ retry: true, delay: 1000 })
  } finally {
    await plugin.close()
  }
})

test("a session on the default model fails over; one whose selection moved on does not", async () => {
  const sessionID = "ses_default"
  const hooks = new Hooks()
  const events = eventStream()
  const switches: string[] = []
  const selection = new Map<"model", typeof failed>()

  const host = fakeHost({
    session: {
      hook: hooks.register,
      switchModel: (input: { readonly model: { readonly id: string } }) => Effect.sync(() => { switches.push(input.model.id) }),
      get: () => Effect.sync(() => ({ model: selection.get("model") })),
    },
    model: { list: () => Effect.succeed(models) },
    tool: { hook: hooks.register },
    event: { subscribe: events.subscribe },
  })

  const plugin = await install(installModelRouter, host, { signal: () => Effect.succeed([
    { type: "model", agent_id: sessionID, model: { provider: "openai", model: "gpt-6-sol" } },
  ]) })

  try {
    await events.publish({ type: "session.execution.started", data: { sessionID } })
    await settle()

    const onDefault = retry(sessionID)

    await hooks.emit("retry", onDefault)
    expect(switches).toEqual(["gpt-6-sol"])
    expect(onDefault.decision).toEqual({ retry: true, delay: 0 })

    selection.set("model", { providerID: "openai", id: "gpt-6-sol", variant: "high" })

    const movedOn = retry(sessionID)

    await hooks.emit("retry", movedOn)
    expect(switches).toHaveLength(1)
    expect(movedOn.decision).toEqual({ retry: true, delay: 1000 })
  } finally {
    await plugin.close()
  }
})

test("stopping while the daemon decides never switches a cancelled execution", async () => {
  const sessionID = "ses_stop"
  const hooks = new Hooks()
  const events = eventStream()
  const switched: string[] = []
  const decided = Effect.runSync(Deferred.make<ReadonlyArray<HostEffect>>())

  const host = fakeHost({
    session: {
      hook: hooks.register,
      switchModel: (input: { readonly model: { readonly id: string } }) => Effect.sync(() => { switched.push(input.model.id) }),
      get: () => Effect.succeed({ model: failed }),
    },
    model: { list: () => Effect.succeed(models) },
    tool: { hook: hooks.register },
    event: { subscribe: events.subscribe },
  })

  const plugin = await install(installModelRouter, host, { signal: () => Deferred.await(decided) })

  try {
    await events.publish({ type: "session.execution.started", data: { sessionID } })
    await settle()
    await hooks.emit("model.request", { sessionID, kind: "primary", model: failed })

    const event = retry(sessionID)
    const pending = hooks.emit("retry", event)

    await settle()
    await events.publish({ type: "session.execution.interrupted", data: { sessionID } })
    await settle()
    Effect.runSync(Deferred.succeed(decided, [{ type: "model", agent_id: sessionID, model: { provider: "openai", model: "gpt-6-sol" } }]))
    await pending

    expect(switched).toHaveLength(0)
    expect(event.decision).toEqual({ retry: true, delay: 1000 })
  } finally {
    await plugin.close()
  }
})

test("running sessions past the cap are forgotten, and a forgotten one keeps the host's retry decision", async () => {
  const hooks = new Hooks()
  const events = eventStream()
  const switches: string[] = []

  const host = fakeHost({
    session: {
      hook: hooks.register,
      switchModel: (input: { readonly model: { readonly id: string } }) => Effect.sync(() => { switches.push(input.model.id) }),
      get: () => Effect.succeed({ model: failed }),
    },
    model: { list: () => Effect.succeed(models) },
    tool: { hook: hooks.register },
    event: { subscribe: events.subscribe },
  })

  const plugin = await install(installModelRouter, host, { signal: (value) => Effect.succeed([
    { type: "model", agent_id: value.agent_id, model: { provider: "openai", model: "gpt-6-sol" } },
  ]) })

  try {
    for (let index = 0; index <= 256; index += 1) {
      await events.publish({ type: "session.execution.started", data: { sessionID: `ses_${index}` } })
    }

    await settle()

    const forgotten = retry("ses_0")
    const latest = retry("ses_256")

    await hooks.emit("retry", forgotten)
    await hooks.emit("retry", latest)

    expect(forgotten.decision).toEqual({ retry: true, delay: 1000 })
    expect(latest.decision).toEqual({ retry: true, delay: 0 })
    expect(switches).toEqual(["gpt-6-sol"])
  } finally {
    await plugin.close()
  }
})
