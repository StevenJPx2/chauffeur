import { expect, test } from "bun:test"
import { Effect } from "effect"
import type { DaemonClient } from "../src/daemon.js"
import type { HostEffect, Signal } from "../src/protocol.js"
import { installToolResults } from "../src/tool-results.js"
import { fakeExposure, fakeHost, Hooks, install, noRulebooks, shippedTexts } from "./support.js"

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
  return { rulebooks: noRulebooks, texts: shippedTexts, signal: (value) => Effect.sync(() => {
    sent.push(value)

    return effects
  }) }
}

test("a missing-tool result sends only registered hidden matches and applies the confirmed reveal", async () => {
  const hooks = new Hooks()
  const sent: Signal[] = []
  const revealed: Array<ReadonlyArray<string>> = []

  const exposure = fakeExposure({
    candidates: () => Effect.succeed([
      { id: "browser_open", description: "Open a browser tab", bytes: 0 },
      { id: "github_merge", description: "Merge a pull request", bytes: 0 },
    ]),
    reveal: (_, names) => Effect.sync(() => {
      revealed.push(names)

      return true
    }),
  })

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

test("an unserializable output is labelled as output, not input", async () => {
  const hooks = new Hooks()
  const sent: Signal[] = []
  const exposure = fakeExposure()
  const plugin = await install(installToolResults(exposure), toolHost(hooks, []), daemon(sent, []))

  await hooks.emit("execute.after", { sessionID, tool: "count", status: "completed", input: { path: "a" }, result: { content: 1n } })

  expect(sent[0]?.kind).toEqual(expect.objectContaining({ input: "{\"path\":\"a\"}", evidence: "unserializable output" }))

  await plugin.close()
})

test("a long output keeps both ends in its evidence, so what comes next survives", async () => {
  const hooks = new Hooks()
  const sent: Signal[] = []
  const plugin = await install(installToolResults(fakeExposure()), toolHost(hooks, []), daemon(sent, []))

  const content = `Saved as #3.\n${"#1 2026-10-05 a memory\n".repeat(200)}Not awake yet. Run: memo wake 2 400`

  await hooks.emit("execute.after", { sessionID, tool: "memo_wake", status: "completed", input: {}, result: { content } })

  const kind = sent[0]?.kind
  const evidence = kind?.type === "tool_result" ? kind.evidence : ""

  expect(Array.from(evidence)).toHaveLength(512)
  expect(evidence.startsWith("\"Saved as #3.")).toBe(true)
  expect(evidence.endsWith("Not awake yet. Run: memo wake 2 400\"")).toBe(true)
  expect(kind).toEqual(expect.objectContaining({ subagent: false }))

  await plugin.close()
})

test("a subagent's tool result says it is a subagent", async () => {
  const hooks = new Hooks()
  const sent: Signal[] = []

  const host = fakeHost({
    tool: { hook: hooks.register },
    session: { get: () => Effect.succeed({ location: { directory: "/projects/app" }, parentID: "ses_parent" }) },
  })

  const plugin = await install(installToolResults(fakeExposure()), host, daemon(sent, []))

  await hooks.emit("execute.after", { sessionID, tool: "read", status: "completed", input: { path: "a" }, result: { content: "x" } })

  expect(sent[0]?.kind).toEqual(expect.objectContaining({ type: "tool_result", workspace: "/projects/app", subagent: true }))

  await plugin.close()
})

test("a steer after a tool result reaches the running turn; prompt context does not", async () => {
  const hooks = new Hooks()
  const delivered: string[] = []
  const exposure = fakeExposure()

  const plugin = await install(installToolResults(exposure), toolHost(hooks, delivered), daemon([], [
    { type: "context", agent_id: sessionID, delivery: "steer", label: "misuse", skills: [], text: "Use gh for GitHub." },
    { type: "context", agent_id: sessionID, delivery: "prompt", label: "pick", skills: [], text: "prompt only" },
  ]))

  await hooks.emit("execute.after", { sessionID, tool: "shell", status: "completed", input: { command: "curl https://api.github.com" }, result: { output: "{}" } })

  expect(delivered).toEqual(["Use gh for GitHub."])

  await plugin.close()
})
