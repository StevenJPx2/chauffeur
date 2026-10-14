import { expect, test } from "bun:test"
import type { SessionMessageAssistant, SessionMessageInfo } from "@opencode/client"
import { Effect, type Schema } from "effect"
import type { DaemonClient } from "../src/daemon.js"
import { Host } from "../src/host.js"
import { installToolDescriptions, rewriteDescriptions } from "../src/tool-descriptions.js"
import { fixedTexts, type HostTexts, SHIPPED_TEXTS, type TextsSource, withTexts } from "../src/texts.js"
import type { Todo } from "../src/todo-data.js"
import { readTodos } from "../src/todos.js"
import { fakeHost, Hooks, install, noRulebooks, shippedTexts } from "./support.js"

const SUBAGENT_GUIDANCE = SHIPPED_TEXTS.subagent.guidance

const daemon: DaemonClient = { rulebooks: noRulebooks, texts: shippedTexts, signal: () => Effect.die("unexpected") }

type ToolPart = Extract<SessionMessageAssistant["content"][number], { type: "tool" }>

function toolCall(todos: ReadonlyArray<Todo>, status: "completed" | "running" = "completed"): ToolPart {
  const input = { todos: todos.map((todo) => ({ content: todo.content, status: todo.status })) }

  const state: ToolPart["state"] = status === "completed"
    ? { status, input, content: [{ type: "text", text: "Todos saved." }] }
    : { status, input, metadata: {} }

  return { type: "tool", id: "call", name: "todowrite", state, time: { created: 0 } }
}

function assistant(content: SessionMessageAssistant["content"]): SessionMessageInfo {
  return { id: "message", time: { created: 0 }, type: "assistant", agent: "build", model: { id: "test", providerID: "test" }, content }
}

/** A host whose storage is a map. */
function hostWithStorage() {
  const stored = new Map<string, Schema.Json>()

  const host = fakeHost({
    storage: {
      get: (key: string) => Effect.sync(() => stored.get(key)),
      set: (key: string, json: Schema.Json) => Effect.sync(() => { stored.set(key, json) }),
    },
  })

  const read = (sessionID: string, messages: ReadonlyArray<SessionMessageInfo>) =>
    // SAFETY: a test transcript carries plain strings where the host brands its IDs; the reader looks only at tool parts.
    Effect.runPromise(readTodos(sessionID, messages as never).pipe(Effect.provideService(Host, host)))

  return { stored, read }
}

test("the list is the last todowrite in the transcript, trimmed and cleaned", async () => {
  const { read } = hostWithStorage()

  const todos = await read("ses_a", [
    assistant([toolCall([{ content: "Old task", status: "pending" }])]),
    assistant([toolCall([
      { content: "  Backfill old rows  ", status: "in_progress" },
      { content: "   ", status: "pending" },
    ])]),
  ])

  expect(todos).toEqual([{ content: "Backfill old rows", status: "in_progress" }])
})

test("an unfinished write does not replace the last good list, and an empty one clears it", async () => {
  const { read } = hostWithStorage()
  const good = assistant([toolCall([{ content: "Current task", status: "pending" }])])

  expect(await read("ses_a", [good, assistant([toolCall([], "running")])])).toEqual([{ content: "Current task", status: "pending" }])
  expect(await read("ses_a", [good, assistant([toolCall([])])])).toEqual([])
})

test("a list the transcript has lost is read back from the last one seen", async () => {
  const { read, stored } = hostWithStorage()

  await read("ses_a", [assistant([toolCall([{ content: "Backfill old rows", status: "in_progress" }])])])

  expect(stored.has("todos/ses_a")).toBe(true)

  // After a compaction the write is no longer in the transcript.
  expect(await read("ses_a", [assistant([])])).toEqual([{ content: "Backfill old rows", status: "in_progress" }])
  expect(await read("ses_b", [assistant([])])).toEqual([])
})

test("a corrupt stored list reads as empty", async () => {
  const { read, stored } = hostWithStorage()

  stored.set("todos/ses_a", [{ content: "x", status: "someday" }])

  expect(await read("ses_a", [])).toEqual([])
})

test("an oversized or malformed write is ignored", async () => {
  const { read } = hostWithStorage()
  const many = Array.from({ length: 65 }, (_, index): Todo => ({ content: `step ${index}`, status: "pending" }))

  expect(await read("ses_a", [assistant([toolCall(many)])])).toEqual([])
})

test("edited wording re-registers while the plugin runs", async () => {
  let texts: HostTexts = SHIPPED_TEXTS
  let version = 0
  const source: TextsSource = { current: () => texts, version: () => version }
  const live: string[] = []

  const register = (current: HostTexts) => Effect.sync(() => {
    live.push(current.labels.context)

    return { dispose: Effect.sync(() => { live.shift() }) }
  })

  const plugin = await install(withTexts(register, "10 millis"), fakeHost({}), daemon, source)
  const settle = () => new Promise((resolve) => setTimeout(resolve, 60))

  try {
    expect(live).toEqual(["Chauffeur {label}"])

    texts = { ...SHIPPED_TEXTS, labels: { ...SHIPPED_TEXTS.labels, context: "Other {label}" } }
    version += 1
    await settle()

    expect(live).toEqual(["Other {label}"])
  } finally {
    await plugin.close()
  }
})

const tool = (description: string) => ({ description, input: {} })

test("the subagent tool's description carries the cheaper-model guidance once", () => {
  const tools = { subagent: tool("Spawns an agent in a child session.") }

  // A hook may see the same definitions twice; the guidance appears once.
  rewriteDescriptions(tools, SHIPPED_TEXTS)
  rewriteDescriptions(tools, SHIPPED_TEXTS)

  expect(tools.subagent.description).toBe(`Spawns an agent in a child session.\n\n${SUBAGENT_GUIDANCE}`)
  expect(SUBAGENT_GUIDANCE).toContain("anthropic/claude-sonnet-5-5")
  expect(SUBAGENT_GUIDANCE).toContain("openai/gpt-6-luna")
})

test("each request's tools take the configured descriptions, and the guidance follows the new subagent text", async () => {
  const hooks = new Hooks()
  const texts = fixedTexts({ ...SHIPPED_TEXTS, tool_descriptions: { execute: "Run code.", subagent: "Start a subagent.", missing: "Ignored." } })
  const plugin = await install(installToolDescriptions, fakeHost({ session: { hook: hooks.register } }), daemon, texts)
  const event = { tools: { execute: tool("A long Code Mode description."), read: tool("Read a file."), subagent: tool("A long subagent description.") } }

  try {
    await hooks.emit("context", event)

    expect(event.tools.execute.description).toBe("Run code.")
    expect(event.tools.read.description).toBe("Read a file.")
    expect(event.tools.subagent.description).toBe(`Start a subagent.\n\n${SUBAGENT_GUIDANCE}`)
  } finally {
    await plugin.close()
  }
})
