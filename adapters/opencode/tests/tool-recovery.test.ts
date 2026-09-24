import { expect, test } from "bun:test"
import type { Plugin } from "@opencode/plugin"
import type { DaemonBridge } from "../src/daemon.js"
import type { ExposureControl } from "../src/exposure.js"
import type { Signal } from "../src/protocol.js"
import { installToolResults } from "../src/tool-results.js"

test("a missing-tool result sends only registered hidden matches and applies the confirmed reveal", async () => {
  const sessionID = "ses_recovery"
  let after: (event: any) => Promise<void> = async () => {}
  const sent: Signal[] = []
  const revealed: string[][] = []
  const ctx = {
    tool: { hook: async (_: string, callback: typeof after) => { after = callback; return { dispose: () => {} } } },
    session: { context: async () => [{ type: "user", text: "Open a browser tab for example.com" }] },
  } as unknown as Plugin.Context
  const exposure = {
    candidates: async () => [
      { id: "browser_open", description: "Open a browser tab", bytes: 0 },
      { id: "github_merge", description: "Merge a pull request", bytes: 0 },
    ],
    reveal: async (_: string, names: string[]) => { revealed.push(names); return true },
  } as ExposureControl
  const daemon = { signal: async (value: Signal) => {
    sent.push(value)
    return [{ type: "tools", agent_id: sessionID, hide: [], reveal: ["browser_open"] }]
  } } as DaemonBridge
  const dispose = await installToolResults(ctx, daemon, exposure)

  await after({ sessionID, tool: "execute", status: "completed", input: { query: "browser_open" }, result: { content: "No matching tool found" } })
  expect(sent[0]?.kind).toEqual(expect.objectContaining({ type: "tool_result", candidates: [
    { id: "browser_open", description: "Open a browser tab", bytes: 0 },
  ] }))
  expect(revealed).toEqual([["browser_open"]])
  await dispose()
})
