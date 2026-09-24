import { expect, test } from "bun:test"
import type { Plugin } from "@opencode/plugin"
import type { DaemonBridge } from "../src/daemon.js"
import { installExposure } from "../src/exposure.js"

test("a recorded mid-turn reveal survives adapter restart and reaches the next tool snapshot", async () => {
  const sessionID = "ses_recovery"
  const hooks = new Map<string, (event: any) => Promise<void> | void>()
  const stored = new Map<string, unknown>([[`hidden/${sessionID}`, ["browser_open"]]])
  const messages: unknown[] = []
  const ctx = {
    session: {
      hook: async (name: string, callback: (event: any) => Promise<void> | void) => {
        hooks.set(name, callback)
        return { dispose: async () => {} }
      },
      context: async () => [],
      synthetic: async (value: unknown) => { messages.push(value) },
    },
    storage: {
      get: async (key: string) => stored.get(key),
      set: async (key: string, value: unknown) => { stored.set(key, value) },
    },
    tool: { list: async () => [{ id: "browser_open", description: "Open a browser tab" }] },
  } as unknown as Plugin.Context
  const daemon = {} as DaemonBridge
  const first = await installExposure(ctx, daemon)

  expect(await first.candidates(sessionID)).toEqual([{ id: "browser_open", description: "Open a browser tab", bytes: 0 }])
  expect(await first.reveal(sessionID, ["browser_open"])).toBe(true)
  expect(messages).toEqual([expect.objectContaining({ metadata: { "chauffeur.hidden": [] }, resume: false })])
  expect(stored.get(`hidden/${sessionID}`)).toEqual([])
  await first.dispose()

  const restarted = await installExposure(ctx, daemon)
  const request = { sessionID, tools: { browser_open: { description: "Open a browser tab" } } }
  await hooks.get("context")?.(request)
  expect(request.tools.browser_open).toBeDefined()
  expect(await restarted.candidates(sessionID)).toEqual([])
  await restarted.dispose()
})

test("a missing registration cannot be revealed", async () => {
  const sessionID = "ses_missing"
  const stored = new Map<string, unknown>([[`hidden/${sessionID}`, ["browser_open"]]])
  const ctx = {
    session: { hook: async () => ({ dispose: async () => {} }), context: async () => [], synthetic: async () => { throw Error("unexpected marker") } },
    storage: { get: async (key: string) => stored.get(key), set: async () => { throw Error("unexpected write") } },
    tool: { list: async () => [] },
  } as unknown as Plugin.Context
  const exposure = await installExposure(ctx, {} as DaemonBridge)

  expect(await exposure.reveal(sessionID, ["browser_open"])).toBe(false)
  expect(stored.get(`hidden/${sessionID}`)).toEqual(["browser_open"])
  await exposure.dispose()
})
