import { expect, test } from "bun:test"
import { Effect } from "effect"
import type { DaemonClient } from "../src/daemon.js"
import { installExposure } from "../src/exposure.js"
import type { Signal } from "../src/protocol.js"
import { fakeHost, Hooks, install } from "./support.js"

const tools = [
  { id: "read", description: "Read a file" },
  { id: "write", description: "Write a file" },
]

/** Sessions ses_a and ses_c run on the host's default agent; ses_b on `plan`. */
const agents = new Map<string, string>([["ses_b", "plan"]])

test("another agent's requests do not change the tools a default-agent session judges", async () => {
  const hooks = new Hooks()
  const stored = new Map<string, ReadonlyArray<string>>()
  const sent: Signal[] = []

  const host = fakeHost({
    session: {
      hook: hooks.register,
      context: () => Effect.succeed([]),
      get: (input: { readonly sessionID: string }) => Effect.succeed({ agent: agents.get(input.sessionID) }),
    },
    storage: {
      get: (key: string) => Effect.succeed(stored.get(key)),
      set: (key: string, value: ReadonlyArray<string>) => Effect.sync(() => { stored.set(key, value) }),
    },
    skill: { list: () => Effect.succeed({ data: [] }) },
    tool: { list: () => Effect.succeed(tools) },
  })

  const daemon: DaemonClient = { signal: (value) => Effect.sync(() => {
    sent.push(value)

    return []
  }) }

  const plugin = await install(installExposure, host, daemon)
  const prompt = (sessionID: string) => hooks.emit("prompt", { sessionID, prompt: { text: "Update the notes" } })

  const request = (sessionID: string, agent: string, names: ReadonlyArray<string>) =>
    hooks.emit("context", { sessionID, agent, tools: Object.fromEntries(names.map((name) => [name, { description: name }])) })

  await prompt("ses_a")
  await request("ses_a", "build", ["read", "write"])
  await prompt("ses_b")
  await request("ses_b", "plan", ["read"])
  await prompt("ses_c")

  const judged = sent.at(-1)?.kind

  expect(judged?.type === "user_message" ? judged.tools.map((tool) => tool.id) : []).toEqual(["read", "write"])

  await plugin.close()
})
