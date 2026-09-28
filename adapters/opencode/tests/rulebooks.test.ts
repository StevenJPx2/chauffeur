import { expect, test } from "bun:test"
import { Effect } from "effect"
import type { DaemonClient } from "../src/daemon.js"
import type { Signal } from "../src/protocol.js"
import { userText } from "../src/text.js"
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

type Recorded = { readonly sent: Signal[]; readonly prompts: Array<{ readonly text: string }>; readonly notes: Array<{ readonly text: string }> }

/** Install the goal command against a daemon answering with `answer`. */
async function goalCommand(answer: { delivery: "resume" | "wait"; text: string }) {
  const recorded: Recorded = { sent: [], prompts: [], notes: [] }
  const commands: Command[] = []

  const host = fakeHost({
    command: { transform: (edit: (editor: { add: (command: Command) => void }) => void) => Effect.sync(() => edit({ add: (command) => { commands.push(command) } })) },
    session: {
      prompt: (message: { readonly text: string }) => Effect.sync(() => { recorded.prompts.push(message) }),
      synthetic: (message: { readonly text: string }) => Effect.sync(() => { recorded.notes.push(message) }),
    },
    skill: { list: () => Effect.succeed({ data: [] }) },
  })

  const daemon: DaemonClient = {
    rulebooks: () => Effect.succeed([{ id: "goal", name: "Goal", description: "Keep working.", args_required: true }]),
    signal: (value) => Effect.sync(() => {
      recorded.sent.push(value)

      return [{ type: "context", agent_id: "ses_goal", label: "Goal", skills: [], ...answer }]
    }),
  }

  const plugin = await install(installRulebooks, host, daemon)
  const run = (text: string) => Effect.runPromise(commands[0]!.execute({ sessionID: "ses_goal", prompt: { text } }))

  return { plugin, commands, recorded, run }
}

test("starting a goal sends the whole text and shows the goal as a prompt", async () => {
  const { plugin, commands, recorded, run } = await goalCommand({ delivery: "resume", text: "Goal: ship it" })
  const long = `ship it ${"and keep the API stable ".repeat(100)}`

  try {
    expect(commands.map((command) => command.name)).toEqual(["goal"])
    expect(commands[0]?.description).toContain("/goal <what>")

    await run(long)

    expect(recorded.sent[0]?.kind).toEqual({ type: "rulebook", command: "start", rulebook: "goal", args: long.trim() })
    expect(recorded.prompts).toEqual([expect.objectContaining({ text: "Goal: ship it" })])
    expect(recorded.notes).toEqual([])
  } finally {
    await plugin.close()
  }
})

test("other answers are Chauffeur notes, and an oversized goal is refused without a signal", async () => {
  const { plugin, recorded, run } = await goalCommand({ delivery: "wait", text: "Goal is running." })

  try {
    await run("")
    expect(recorded.notes).toEqual([expect.objectContaining({ text: "Goal is running." })])
    expect(recorded.prompts).toEqual([])

    await run("x".repeat(70_000))
    expect(recorded.sent).toHaveLength(1)
    expect(recorded.notes[1]?.text).toContain("the limit is 64 KiB")
  } finally {
    await plugin.close()
  }
})

test("the user's words are sent whole, or left out when over the engine's bound", () => {
  const words = "é".repeat(20_000)

  expect(userText(words)).toBe(words)
  expect(userText("x".repeat(65_537))).toBe("")
})

test("no rulebooks, no commands", async () => {
  let transformed = false
  const host = fakeHost({ command: { transform: () => Effect.sync(() => { transformed = true }) } })
  const daemon: DaemonClient = { rulebooks: () => Effect.succeed([]), signal: () => Effect.die("unexpected") }
  const plugin = await install(installRulebooks, host, daemon)

  expect(transformed).toBe(false)
  await plugin.close()
})
