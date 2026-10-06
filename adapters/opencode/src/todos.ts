import { Effect, Option, Schema, type Scope } from "effect"
import { Host } from "./host.js"
import { clipBytes } from "./text.js"
import { fill, type HostTexts, Texts, withTexts } from "./texts.js"

/** Chauffeur's own todo tool, in place of V1's `todowrite`; exposure never hides it. */
export const TODO_TOOL = "todowrite"

/** The engine's bounds: at most 64 todos of 1 to 512 bytes each. */
const MAX_TODOS = 64

const TODO_BYTES = 512

const Status = Schema.Literals(["pending", "in_progress", "completed", "cancelled"])

const Todo = Schema.Struct({ content: Schema.String, status: Status })

/** One item of a session's todo list, as the engine reads it. */
export type Todo = typeof Todo.Type

const Todos = Schema.Array(Todo)

/** The tool's input, its fields described in the texts' words. */
function inputSchema(texts: HostTexts) {
  const item = Schema.Struct({
    content: Schema.String.annotate({ description: texts.todowrite.content }),
    status: Status.annotate({ description: texts.todowrite.status }),
  })

  return Schema.Struct({ todos: Schema.Array(item).annotate({ description: texts.todowrite.todos }) })
}

const storageKey = (sessionID: string): string => `todos/${sessionID}`

/** Register `todowrite`: the list is kept per session and reported at each turn end. */
export const installTodos: Effect.Effect<void, never, Host | Texts | Scope.Scope> = Effect.gen(function* () {
  const host = yield* Host

  yield* withTexts((texts) => host.tool.transform((editor) => {
    editor.add({
      name: TODO_TOOL,
      description: texts.todowrite.description,
      input: inputSchema(texts),
      options: { codemode: false },
      execute: ({ todos }, context) =>
        write(String(context.sessionID), todos, texts).pipe(
          Effect.provideService(Host, host),
          Effect.map((content) => ({ content })),
        ),
    })
  }))
})

function write(sessionID: string, todos: ReadonlyArray<Todo>, texts: HostTexts): Effect.Effect<string, never, Host> {
  return Effect.gen(function* () {
    const host = yield* Host

    if (todos.length > MAX_TODOS) return fill(texts.todowrite.too_many, { max: String(MAX_TODOS) })

    // Blank items are dropped; long ones are clipped to the engine's bound.
    const kept = todos.flatMap((todo) => {
      const content = clipBytes(todo.content.trim(), TODO_BYTES)

      return content === "" ? [] : [{ content, status: todo.status }]
    })

    yield* host.storage.set(storageKey(sessionID), kept)

    return render(kept, texts)
  })
}

/** The session's todo list as last written; empty when none or unreadable. */
export function readTodos(sessionID: string): Effect.Effect<ReadonlyArray<Todo>, never, Host> {
  return Effect.gen(function* () {
    const host = yield* Host
    const stored = yield* host.storage.get(storageKey(sessionID))

    return Option.getOrElse(Schema.decodeUnknownOption(Todos)(stored), (): ReadonlyArray<Todo> => [])
  })
}

const MARKS: Record<Todo["status"], string> = {
  pending: "[ ]",
  in_progress: "[>]",
  completed: "[x]",
  cancelled: "[-]",
}

/** The list as the agent reads it back. */
export function render(todos: ReadonlyArray<Todo>, texts: HostTexts): string {
  if (todos.length === 0) return texts.todowrite.empty

  const open = todos.filter((todo) => todo.status === "pending" || todo.status === "in_progress").length
  const lines = todos.map((todo) => `${MARKS[todo.status]} ${todo.content}`)

  return [fill(texts.todowrite.heading, { open: String(open) }), ...lines].join("\n")
}
