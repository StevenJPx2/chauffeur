import { expect, test } from "bun:test"
import type { Plugin } from "@opencode/plugin"
import type { DaemonBridge } from "../src/daemon.js"
import type { Effect, Signal } from "../src/protocol.js"
import { installModelRouter } from "../src/model-router.js"

test("a tool in an earlier request does not block failover; a tool in this request does", async () => {
  const hooks = new Map<string, (event: any) => Promise<void> | void>()
  const signals: Signal[] = []
  const switches: unknown[] = []
  const sessionID = "ses_test"
  const failed = { providerID: "anthropic", id: "claude-opus-5-5", variant: "high" }
  const next = { provider: "openai", model: "gpt-6-sol" }
  const ctx = {
    session: {
      hook: async (name: string, callback: (event: any) => Promise<void> | void) => {
        hooks.set(name, callback)
        return { dispose: async () => {} }
      },
      switchModel: async (input: unknown) => { switches.push(input) },
      get: async () => ({ model: failed }),
    },
    model: { list: async () => ({ data: [
      { ...failed, enabled: true, status: "active" },
      { providerID: "openai", id: "gpt-6-sol", enabled: true, status: "active" },
    ] }) },
    tool: { hook: async (name: string, callback: (event: any) => void) => {
      hooks.set(name, callback)
      return { dispose: async () => {} }
    } },
    event: { subscribe: async function* ({ signal }: { signal: AbortSignal }) {
      yield { type: "session.execution.started", data: { sessionID } }
      if (!signal.aborted) await new Promise<void>((resolve) => signal.addEventListener("abort", () => resolve(), { once: true }))
    } },
  } as unknown as Plugin.Context
  const daemon = { signal: async (value: Signal): Promise<Effect[]> => {
    signals.push(value)
    return value.kind.type === "model_error" && !value.kind.tool_executed
      ? [{ type: "model", agent_id: sessionID, model: next }]
      : [{ type: "model", agent_id: sessionID, model: null }]
  } } as DaemonBridge
  const stop = await installModelRouter(ctx, daemon)
  const emit = async (name: string, event: object) => { await hooks.get(name)?.(event) }
  const retry = () => ({ sessionID, model: failed, error: { type: "provider.rate_limit", status: 429, message: "limited" }, decision: { retry: true, delay: 1000 } })

  try {
    await emit("model.request", { sessionID, kind: "primary", model: failed })
    await emit("execute.after", { sessionID })
    await emit("model.request", { sessionID, kind: "primary", model: failed })
    const safe = retry()
    await emit("retry", safe)
    expect(signals.at(-1)?.kind).toMatchObject({ tool_executed: false })
    expect(switches).toHaveLength(1)
    expect(safe.decision).toEqual({ retry: true, delay: 0 })

    await emit("model.request", { sessionID, kind: "primary", model: failed })
    await emit("execute.after", { sessionID })
    const unsafe = retry()
    await emit("retry", unsafe)
    expect(signals.at(-1)?.kind).toMatchObject({ tool_executed: true })
    expect(switches).toHaveLength(1)
    expect(unsafe.decision).toEqual({ retry: true, delay: 1000 })
  } finally {
    await stop()
  }
})

test("stopping while the daemon decides never switches a cancelled execution", async () => {
  const hooks = new Map<string, (event: any) => Promise<void> | void>()
  const sessionID = "ses_stop"
  const failed = { providerID: "anthropic", id: "claude-opus-5-5", variant: "high" }
  let publish: (event: { type: string; data: { sessionID: string } }) => void = () => {}
  let release: (effects: Effect[]) => void = () => {}
  const switched: unknown[] = []
  const ctx = {
    session: {
      hook: async (name: string, callback: (event: any) => Promise<void> | void) => {
        hooks.set(name, callback)
        return { dispose: async () => {} }
      },
      switchModel: async (model: unknown) => { switched.push(model) },
      get: async () => ({ model: failed }),
    },
    model: { list: async () => ({ data: [
      { ...failed, enabled: true, status: "active" },
      { providerID: "openai", id: "gpt-6-sol", enabled: true, status: "active" },
    ] }) },
    tool: { hook: async () => ({ dispose: async () => {} }) },
    event: { subscribe: async function* ({ signal }: { signal: AbortSignal }) {
      yield { type: "session.execution.started", data: { sessionID } }
      while (!signal.aborted) {
        const event = await new Promise<{ type: string; data: { sessionID: string } }>((resolve) => { publish = resolve })
        yield event
      }
    } },
  } as unknown as Plugin.Context
  const daemon = { signal: () => new Promise<Effect[]>((resolve) => { release = resolve }) } as DaemonBridge
  const stop = await installModelRouter(ctx, daemon)

  try {
    await hooks.get("model.request")?.({ sessionID, kind: "primary", model: failed })
    const retry = { sessionID, model: failed, error: { type: "provider.rate_limit", status: 429, message: "limited" }, decision: { retry: true, delay: 1000 } }
    const pending = hooks.get("retry")?.(retry)
    await Promise.resolve()
    publish({ type: "session.execution.interrupted", data: { sessionID } })
    await new Promise<void>((resolve) => setImmediate(resolve))
    release([{ type: "model", agent_id: sessionID, model: { provider: "openai", model: "gpt-6-sol" } }])
    await pending

    expect(switched).toHaveLength(0)
    expect(retry.decision).toEqual({ retry: true, delay: 1000 })
  } finally {
    await stop()
  }
})
