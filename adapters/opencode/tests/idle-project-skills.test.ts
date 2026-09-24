import { expect, test } from "bun:test"
import { Effect } from "effect"
import type { DaemonClient } from "../src/daemon.js"
import { installIdle } from "../src/idle.js"
import type { Signal } from "../src/protocol.js"
import { eventStream, fakeHost, install, settle } from "./support.js"

test("turn end carries the workspace and user instruction, and delivers a confirmed reminder", async () => {
  const sessionID = "ses_hpdp"
  const events = eventStream()
  const sent: Signal[] = []
  const messages: Array<{ readonly text: string; readonly resume?: boolean }> = []

  const host = fakeHost({
    event: { subscribe: events.subscribe },
    session: {
      get: () => Effect.succeed({ location: { directory: "/projects/hpdp-overlay/main" } }),
      context: () => Effect.succeed([{ type: "user", text: "Do not open a PR", metadata: {} }]),
      synthetic: (message: (typeof messages)[number]) => Effect.sync(() => { messages.push(message) }),
    },
    skill: { list: () => Effect.succeed({ data: [] }) },
  })

  const daemon: DaemonClient = { signal: (value) => Effect.sync(() => {
    sent.push(value)

    return [{ type: "context", agent_id: sessionID, delivery: "resume", label: "HPDP", skills: [], text: "Check the overlay" }]
  }) }

  const plugin = await install(installIdle, host, daemon)

  try {
    await events.publish({ type: "session.execution.succeeded", data: { sessionID } })
    await settle()

    expect(sent[0]?.kind).toEqual({ type: "turn_end", workspace: "/projects/hpdp-overlay/main", user_request: "Do not open a PR" })
    expect(messages).toEqual([expect.objectContaining({ text: "Check the overlay", resume: true })])
  } finally {
    await plugin.close()
  }
})
