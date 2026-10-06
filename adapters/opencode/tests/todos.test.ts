import { expect, test } from "bun:test"
import { Effect, type Schema } from "effect"
import type { DaemonClient } from "../src/daemon.js"
import { Host } from "../src/host.js"
import { installSubagentGuidance } from "../src/subagents.js"
import { type HostTexts, SHIPPED_TEXTS, type TextsSource, withTexts } from "../src/texts.js"
import { installTodos, readTodos, render, TODO_TOOL, type Todo } from "../src/todos.js"
import { fakeHost, install, noRulebooks, shippedTexts } from "./support.js"

const SUBAGENT_GUIDANCE = SHIPPED_TEXTS.subagent.guidance

type Execute = (input: { readonly todos: ReadonlyArray<Todo> }, context: { readonly sessionID: string }) => Effect.Effect<{ content: string }>

const daemon: DaemonClient = { rulebooks: noRulebooks, texts: shippedTexts, signal: () => Effect.die("unexpected") }

/** A host whose storage is a map, and whose added tools are captured. */
function hostWithStorage() {
  const stored = new Map<string, Schema.Json>()
  const tools = new Map<string, Execute>()

  const host = fakeHost({
    storage: {
      get: (key: string) => Effect.sync(() => stored.get(key)),
      set: (key: string, json: Schema.Json) => Effect.sync(() => { stored.set(key, json) }),
    },
    tool: {
      transform: (edit: (editor: { add: (tool: { name: string; execute: Execute }) => void }) => void) => Effect.sync(() => {
        edit({ add: (tool) => { tools.set(tool.name, tool.execute) } })

        return { dispose: Effect.void }
      }),
    },
  })

  const read = (sessionID: string) => Effect.runPromise(readTodos(sessionID).pipe(Effect.provideService(Host, host)))

  const write = (sessionID: string, todos: ReadonlyArray<Todo>) => {
    const execute = tools.get(TODO_TOOL)

    if (!execute) throw new Error("todowrite was not registered")

    return Effect.runPromise(execute({ todos }, { sessionID })).then((reply) => reply.content)
  }

  return { host, stored, read, write }
}

test("todowrite keeps the session's list and reads it back", async () => {
  const { host, read, write } = hostWithStorage()
  const plugin = await install(installTodos, host, daemon)

  try {
    const reply = await write("ses_a", [
      { content: "Write the migration", status: "completed" },
      { content: "  Backfill old rows  ", status: "in_progress" },
      { content: "   ", status: "pending" },
    ])

    expect(reply).toBe("Todos (1 open):\n[x] Write the migration\n[>] Backfill old rows")
    expect(await read("ses_a")).toEqual([
      { content: "Write the migration", status: "completed" },
      { content: "Backfill old rows", status: "in_progress" },
    ])
    expect(await read("ses_b")).toEqual([])
  } finally {
    await plugin.close()
  }
})

test("an oversized list is refused and a corrupt stored list reads as empty", async () => {
  const { host, stored, read, write } = hostWithStorage()
  const plugin = await install(installTodos, host, daemon)

  try {
    const many = Array.from({ length: 65 }, (_, index): Todo => ({ content: `step ${index}`, status: "pending" }))

    expect(await write("ses_a", many)).toContain("at most 64")
    expect(stored.has("todos/ses_a")).toBe(false)

    stored.set("todos/ses_a", [{ content: "x", status: "someday" }])

    expect(await read("ses_a")).toEqual([])
  } finally {
    await plugin.close()
  }
})

test("an empty list renders as empty", () => {
  expect(render([], SHIPPED_TEXTS)).toBe("The todo list is empty.")
})

test("edited wording re-registers the tool while the plugin runs", async () => {
  let texts: HostTexts = SHIPPED_TEXTS
  let version = 0
  const source: TextsSource = { current: () => texts, version: () => version }
  const live: string[] = []

  const register = (current: HostTexts) => Effect.sync(() => {
    live.push(current.todowrite.empty)

    return { dispose: Effect.sync(() => { live.shift() }) }
  })

  const plugin = await install(withTexts(register, "10 millis"), fakeHost({}), daemon, source)
  const settle = () => new Promise((resolve) => setTimeout(resolve, 60))

  try {
    expect(live).toEqual(["The todo list is empty."])

    texts = { ...SHIPPED_TEXTS, todowrite: { ...SHIPPED_TEXTS.todowrite, empty: "Nothing to do." } }
    version += 1
    await settle()

    expect(live).toEqual(["Nothing to do."])
  } finally {
    await plugin.close()
  }
})

test("the subagent tool's description carries the cheaper-model guidance once", async () => {
  type Described = { description: string }

  type Editor = { update: (id: string, change: (tool: Described) => void) => void }

  const subagent: Described = { description: "Spawns an agent in a child session." }
  const editor: Editor = { update: (id, change) => { if (id === "subagent") change(subagent) } }

  const host = fakeHost({
    tool: {
      transform: (edit: (editor: Editor) => void) => Effect.sync(() => {
        // The host may run a transform more than once; the guidance appears once.
        edit(editor)
        edit(editor)

        return { dispose: Effect.void }
      }),
    },
  })

  const plugin = await install(installSubagentGuidance, host, daemon)

  try {
    expect(subagent.description).toBe(`Spawns an agent in a child session.\n\n${SUBAGENT_GUIDANCE}`)
    expect(SUBAGENT_GUIDANCE).toContain("anthropic/claude-sonnet-5-5")
    expect(SUBAGENT_GUIDANCE).toContain("openai/gpt-6-luna")
  } finally {
    await plugin.close()
  }
})
