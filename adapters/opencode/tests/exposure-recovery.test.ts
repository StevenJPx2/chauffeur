import { expect, test } from "bun:test"
import { Effect } from "effect"
import { installExposure } from "../src/exposure.js"
import { fakeHost, Hooks, install, noDaemon, sessionID as session } from "./support.js"

const browser = { id: "browser_open", description: "Open a browser tab" }

test("a recorded mid-turn reveal survives adapter restart and reaches the next tool snapshot", async () => {
  const sessionID = "ses_recovery"
  const hooks = new Hooks()
  const stored = new Map<string, ReadonlyArray<string>>([[`hidden/${sessionID}`, ["browser_open"]]])
  const markers: Array<{ readonly metadata?: { readonly [key: string]: ReadonlyArray<string> }; readonly resume?: boolean }> = []

  const host = fakeHost({
    session: {
      hook: hooks.register,
      context: () => Effect.succeed([]),
      synthetic: (message: (typeof markers)[number]) => Effect.sync(() => { markers.push(message) }),
    },
    storage: {
      get: (key: string) => Effect.succeed(stored.get(key)),
      set: (key: string, value: ReadonlyArray<string>) => Effect.sync(() => { stored.set(key, value) }),
    },
    tool: { list: () => Effect.succeed([browser]) },
  })

  const first = await install(installExposure, host, noDaemon)

  expect(await Effect.runPromise(first.value.candidates(session(sessionID)))).toEqual([{ ...browser, bytes: 0 }])
  expect(await Effect.runPromise(first.value.reveal(session(sessionID), ["browser_open"]))).toBe(true)
  expect(markers).toEqual([expect.objectContaining({ metadata: { "chauffeur.hidden": [] }, resume: false })])
  expect(stored.get(`hidden/${sessionID}`)).toEqual([])

  await first.close()

  const restarted = await install(installExposure, host, noDaemon)
  const request = { sessionID, tools: { browser_open: { description: browser.description } } }

  await hooks.emit("context", request)

  expect(request.tools.browser_open).toBeDefined()
  expect(await Effect.runPromise(restarted.value.candidates(session(sessionID)))).toEqual([])

  await restarted.close()
})

test("a missing registration cannot be revealed", async () => {
  const sessionID = "ses_missing"
  const hooks = new Hooks()
  const stored = new Map<string, ReadonlyArray<string>>([[`hidden/${sessionID}`, ["browser_open"]]])

  const host = fakeHost({
    session: { hook: hooks.register, context: () => Effect.succeed([]), synthetic: () => Effect.die("unexpected marker") },
    storage: { get: (key: string) => Effect.succeed(stored.get(key)), set: () => Effect.die("unexpected write") },
    tool: { list: () => Effect.succeed([]) },
  })

  const exposure = await install(installExposure, host, noDaemon)

  expect(await Effect.runPromise(exposure.value.reveal(session(sessionID), ["browser_open"]))).toBe(false)
  expect(stored.get(`hidden/${sessionID}`)).toEqual(["browser_open"])

  await exposure.close()
})
