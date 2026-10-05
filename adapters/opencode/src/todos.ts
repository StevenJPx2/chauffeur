import { Effect, Option, Schema, type Scope } from "effect"
import { Host } from "./host.js"
import { clipBytes } from "./text.js"

/** Chauffeur's own todo tool, in place of V1's `todowrite`; exposure never hides it. */
export const TODO_TOOL = "todowrite"

/** The engine's bounds: at most 64 todos of 1 to 512 bytes each. */
const MAX_TODOS = 64

const TODO_BYTES = 512

const Status = Schema.Literals(["pending", "in_progress", "completed", "cancelled"])

const Todo = Schema.Struct({
  content: Schema.String.annotate({ description: "One distinct step, in a short sentence." }),
  status: Status.annotate({ description: "pending, in_progress (one at a time), completed once its evidence is in, or cancelled." }),
})

/** One item of a session's todo list, as the engine reads it. */
export type Todo = typeof Todo.Type

const Todos = Schema.Array(Todo)

const Input = Schema.Struct({
  todos: Todos.annotate({ description: "The whole list, in order. Each call replaces the previous list." }),
})

const DESCRIPTION = [
  "Write the todo list for this session: the distinct steps of the current task and where each stands.",
  "Use it for work with several steps: write the list before starting, keep one todo in_progress, mark each completed only when its evidence is in, and cancel any that no longer apply.",
  "Each call replaces the whole list. Chauffeur keeps a /goal going while any todo is pending or in progress.",
].join(" ")

const storageKey = (sessionID: string): string => `todos/${sessionID}`

/** Register `todowrite`: the list is kept per session and reported at each turn end. */
export const installTodos: Effect.Effect<void, never, Host | Scope.Scope> = Effect.gen(function* () {
  const host = yield* Host

  yield* host.tool.transform((editor) => {
    editor.add({
      name: TODO_TOOL,
      description: DESCRIPTION,
      input: Input,
      options: { codemode: false },
      execute: ({ todos }, context) =>
        write(String(context.sessionID), todos).pipe(
          Effect.provideService(Host, host),
          Effect.map((content) => ({ content })),
        ),
    })
  })
})

function write(sessionID: string, todos: ReadonlyArray<Todo>): Effect.Effect<string, never, Host> {
  return Effect.gen(function* () {
    const host = yield* Host

    if (todos.length > MAX_TODOS) return `A todo list holds at most ${MAX_TODOS} items; merge some and write it again.`

    // Blank items are dropped; long ones are clipped to the engine's bound.
    const kept = todos.flatMap((todo) => {
      const content = clipBytes(todo.content.trim(), TODO_BYTES)

      return content === "" ? [] : [{ content, status: todo.status }]
    })

    yield* host.storage.set(storageKey(sessionID), kept)

    return render(kept)
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
export function render(todos: ReadonlyArray<Todo>): string {
  if (todos.length === 0) return "The todo list is empty."

  const open = todos.filter((todo) => todo.status === "pending" || todo.status === "in_progress").length
  const lines = todos.map((todo) => `${MARKS[todo.status]} ${todo.content}`)

  return [`Todos (${open} open):`, ...lines].join("\n")
}
