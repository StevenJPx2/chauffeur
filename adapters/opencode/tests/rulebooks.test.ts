import { expect, test } from "bun:test"
import { Effect } from "effect"
import type { DaemonClient } from "../src/daemon.js"
import type { Signal } from "../src/protocol.js"
import { installRulebooks, parseRulebookInput } from "../src/rulebooks.js"
import { fakeHost, install } from "./support.js"

test("command text maps to a rulebook command", () => {
  expect(parseRulebookInput("")).toEqual({ command: "status", args: "" })
  expect(parseRulebookInput(" Pause ")).toEqual({ command: "pause", args: "" })
  expect(parseRulebookInput("resume")).toEqual({ command: "resume", args: "" })
  expect(parseRulebookInput("clear")).toEqual({ command: "clear", args: "" })
  expect(parseRulebookInput("p95 under 120 ms")).toEqual({ command: "start", args: "p95 under 120 ms" })
  expect(parseRulebookInput("pause the queue when idle")).toEqual({ command: "start", args: "pause the queue when idle" })
})

type Command = {
  readonly name: string
  readonly description?: string
  readonly execute: (input: { sessionID: string; prompt: { text: string } }) => Effect.Effect<void, unknown>
}

test("each rulebook becomes a command that signals the engine and delivers its answer", async () => {
  const sent: Signal[] = []
  const messages: Array<{ readonly text: string; readonly resume?: boolean }> = []
  const commands: Command[] = []

  const host = fakeHost({
    command: { transform: (edit: (editor: { add: (command: Command) => void }) => void) => Effect.sync(() => edit({ add: (command) => { commands.push(command) } })) },
    session: { synthetic: (message: (typeof messages)[number]) => Effect.sync(() => { messages.push(message) }) },
    skill: { list: () => Effect.succeed({ data: [] }) },
  })

  const daemon: DaemonClient = {
    rulebooks: () => Effect.succeed([{ id: "goal", name: "Goal", description: "Keep working.", args_required: true }]),
    signal: (value) => Effect.sync(() => {
      sent.push(value)

      return [{ type: "context", agent_id: "ses_goal", delivery: "resume", label: "Goal", skills: [], text: "Goal: ship it" }]
    }),
  }

  const plugin = await install(installRulebooks, host, daemon)

  try {
    expect(commands.map((command) => command.name)).toEqual(["goal"])
    expect(commands[0]?.description).toContain("/goal <what>")

    await Effect.runPromise(commands[0]!.execute({ sessionID: "ses_goal", prompt: { text: "ship it" } }))

    expect(sent[0]?.kind).toEqual({ type: "rulebook", command: "start", rulebook: "goal", args: "ship it" })
    expect(messages).toEqual([expect.objectContaining({ text: "Goal: ship it", resume: true })])
  } finally {
    await plugin.close()
  }
})

test("no rulebooks, no commands", async () => {
  let transformed = false
  const host = fakeHost({ command: { transform: () => Effect.sync(() => { transformed = true }) } })
  const daemon: DaemonClient = { rulebooks: () => Effect.succeed([]), signal: () => Effect.die("unexpected") }
  const plugin = await install(installRulebooks, host, daemon)

  expect(transformed).toBe(false)
  await plugin.close()
})
