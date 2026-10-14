import { Option, Schema } from "effect"
import type { HistoryMessage } from "./host.js"
import { clipBytes } from "./text.js"

type ToolState = Extract<Extract<HistoryMessage, { type: "assistant" }>["content"][number], { type: "tool" }>["state"]

/** What a finished tool call recorded as its input. */
type RecordedInput = Extract<ToolState, { status: "completed" }>["input"]

const Status = Schema.Literals(["pending", "in_progress", "completed", "cancelled"])

const TodoSchema = Schema.Struct({ content: Schema.String, status: Status })

const TodoInputSchema = Schema.Struct({ todos: Schema.Array(TodoSchema) })

/** A stored list, as the engine reads it. */
export const TodosSchema = Schema.Array(TodoSchema)

export type Todo = typeof TodoSchema.Type

/** The tool the agent writes its todo list with; exposure never judges it. */
export const TODO_TOOL = "todowrite"

/** The engine's bounds: at most 64 todos of 1 to 512 bytes each. */
const MAX_TODOS = 64

const TODO_BYTES = 512

/** Decode and clean the `todowrite` input recorded in a session message. */
export function readTodoInput(value: RecordedInput): Todo[] | undefined {
  const input = Schema.decodeUnknownOption(TodoInputSchema)(value)

  if (Option.isNone(input) || input.value.todos.length > MAX_TODOS) return undefined

  return input.value.todos.flatMap((todo) => {
    const content = clipBytes(todo.content.trim(), TODO_BYTES)

    return content === "" ? [] : [{ content, status: todo.status }]
  })
}
