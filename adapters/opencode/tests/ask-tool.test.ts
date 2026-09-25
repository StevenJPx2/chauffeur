import { expect, test } from "bun:test"
import { Effect } from "effect"
import { ASK_TOOL, installAskTool } from "../src/ask-tool.js"
import { type DaemonClient, DaemonError } from "../src/daemon.js"
import type { ExposureControl } from "../src/exposure.js"
import type { HostEffect, Signal } from "../src/protocol.js"
import { fakeExposure, fakeHost, install } from "./support.js"

const sessionID = "ses_ask"

/** The host has decoded the call's input by the time `execute` runs. */
type Need = { readonly need: string }

type Registered = {
  readonly name: string
  readonly options?: { readonly codemode?: boolean }
  readonly execute: (input: Need, context: { readonly sessionID: string; readonly agent: string }) => Effect.Effect<{ readonly content: string }>
}

/** Install the tool and return its registered definition. */
async function registered(daemon: DaemonClient, exposure: ExposureControl) {
  const tools: Registered[] = []

  const host = fakeHost({
    tool: {
      transform: (callback: (editor: { add: (tool: Registered) => void }) => void) => Effect.sync(() => {
        callback({ add: (tool) => { tools.push(tool) } })

        return { dispose: Effect.void }
      }),
    },
    session: {
      context: () => Effect.succeed([
        { type: "user", text: "Check the landing page renders", metadata: {} },
        { type: "user", text: "CI failed", metadata: { sourcefed: { kind: "ci" } } },
      ]),
    },
    skill: { list: () => Effect.succeed({ data: [{ id: "jira-cli", content: "Use jira issue create." }] }) },
  })

  const plugin = await install(installAskTool(exposure), host, daemon)

  const [tool] = tools

  if (tool === undefined) throw new Error("ask_chauffeur was not registered")

  const ask = (need: Need) => Effect.runPromise(tool.execute(need, { sessionID, agent: "build" })).then((result) => result.content)

  return { tool, ask, close: plugin.close }
}

function replying(sent: Signal[], effects: ReadonlyArray<HostEffect>): DaemonClient {
  return { signal: (value) => Effect.sync(() => {
    sent.push(value)

    return effects
  }) }
}

test("a request carries the need, the user's words, hidden tools, and Code Mode, and replies with what was granted", async () => {
  const sent: Signal[] = []
  const revealed: Array<ReadonlyArray<string>> = []

  const exposure = fakeExposure({
    candidates: () => Effect.succeed([{ id: "browser_open", description: "Open a tab", bytes: 0 }]),
    reveal: (_, names) => Effect.sync(() => {
      revealed.push(names)

      return true
    }),
    codeMode: (agent, text) => Effect.succeed([{ name: `safari-for-${agent}`, size: 3, tools: [{ id: text.slice(0, 5), description: "", bytes: 0 }] }]),
  })

  const { tool, ask, close } = await registered(replying(sent, [
    { type: "tools", agent_id: sessionID, hide: [], reveal: ["browser_open"] },
    { type: "context", agent_id: sessionID, delivery: "steer", label: "skill jira-cli", skills: ["jira-cli"], text: null },
    { type: "context", agent_id: "ses_other", delivery: "steer", label: "x", skills: [], text: "not for this session" },
  ]), exposure)

  const reply = await ask({ need: "drive a browser" })

  expect(tool).toMatchObject({ name: ASK_TOOL, options: { codemode: false } })
  expect(sent[0]?.kind).toEqual({
    type: "agent_request",
    need: "drive a browser",
    user_request: "Check the landing page renders",
    tools: [{ id: "browser_open", description: "Open a tab", bytes: 0 }],
    code_mode: [{ name: "safari-for-build", size: 3, tools: [{ id: "drive", description: "", bytes: 0 }] }],
  })
  expect(revealed).toEqual([["browser_open"]])
  expect(reply).toContain("revealed these tools; they are available from your next step: browser_open")
  expect(reply).toContain("Use jira issue create.")
  expect(reply).not.toContain("not for this session")

  await close()
})

test("nothing granted, a failed reveal, or a daemon failure still replies", async () => {
  const nothing = await registered(replying([], []), fakeExposure())
  const unrevealed = await registered(replying([], [{ type: "tools", agent_id: sessionID, hide: [], reveal: ["browser_open"] }]), fakeExposure())
  const down = await registered({ signal: () => Effect.fail(new DaemonError({ message: "daemon down" })) }, fakeExposure())

  expect(await nothing.ask({ need: "a time machine" })).toContain("found no hidden tool")
  expect(await unrevealed.ask({ need: "a browser" })).toContain("found no hidden tool")
  expect(await down.ask({ need: "a browser" })).toContain("could not answer")

  await Promise.all([nothing.close(), unrevealed.close(), down.close()])
})
