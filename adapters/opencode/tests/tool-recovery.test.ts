import { expect, test } from "bun:test"
import { Effect } from "effect"
import type { DaemonClient } from "../src/daemon.js"
import type { ExposureControl } from "../src/exposure.js"
import type { HostEffect, Signal } from "../src/protocol.js"
import { installToolResults } from "../src/tool-results.js"
import { fakeHost, Hooks, install } from "./support.js"

const sessionID = "ses_recovery"

function toolHost(hooks: Hooks, delivered: string[]) {
  return fakeHost({
    tool: { hook: hooks.register },
    session: {
      context: () => Effect.succeed([{ type: "user", text: "Open a browser tab for example.com" }]),
      get: () => Effect.succeed({ location: { directory: "/projects/chauffeur" } }),
      synthetic: (message: { readonly text: string }) => Effect.sync(() => { delivered.push(message.text) }),
    },
    skill: { list: () => Effect.succeed({ data: [] }) },
  })
}

function daemon(sent: Signal[], effects: ReadonlyArray<HostEffect>): DaemonClient {
  return { signal: (value) => Effect.sync(() => {
    sent.push(value)

    return effects
  }) }
}

test("a missing-tool result sends only registered hidden matches and applies the confirmed reveal", async () => {
  const hooks = new Hooks()
  const sent: Signal[] = []
  const revealed: Array<ReadonlyArray<string>> = []

  const exposure: ExposureControl = {
    candidates: () => Effect.succeed([
      { id: "browser_open", description: "Open a browser tab", bytes: 0 },
      { id: "github_merge", description: "Merge a pull request", bytes: 0 },
    ]),
    reveal: (_, names) => Effect.sync(() => {
      revealed.push(names)

      return true
    }),
  }

  const plugin = await install(
    installToolResults(exposure),
    toolHost(hooks, []),
    daemon(sent, [{ type: "tools", agent_id: sessionID, hide: [], reveal: ["browser_open"] }]),
  )

  await hooks.emit("execute.after", { sessionID, tool: "execute", status: "completed", input: { query: "browser_open" }, result: { content: "No matching tool found" } })

  expect(sent[0]?.kind).toEqual(expect.objectContaining({ type: "tool_result", workspace: "/projects/chauffeur", candidates: [
    { id: "browser_open", description: "Open a browser tab", bytes: 0 },
  ] }))
  expect(revealed).toEqual([["browser_open"]])

  await plugin.close()
})

test("a steer after a tool result reaches the running turn; prompt context does not", async () => {
  const hooks = new Hooks()
  const delivered: string[] = []
  const exposure: ExposureControl = { candidates: () => Effect.succeed([]), reveal: () => Effect.succeed(false) }

  const plugin = await install(installToolResults(exposure), toolHost(hooks, delivered), daemon([], [
    { type: "context", agent_id: sessionID, delivery: "steer", label: "misuse", skills: [], text: "Use gh for GitHub." },
    { type: "context", agent_id: sessionID, delivery: "prompt", label: "pick", skills: [], text: "prompt only" },
  ]))

  await hooks.emit("execute.after", { sessionID, tool: "shell", status: "completed", input: { command: "curl https://api.github.com" }, result: { output: "{}" } })

  expect(delivered).toEqual(["Use gh for GitHub."])

  await plugin.close()
})
