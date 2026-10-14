import type { HistoryMessage } from "./host.js"
import { readTodoInput, TODO_TOOL, type Todo } from "./todo-data.js"

/** The last successful `todowrite` call in the transcript, which is oldest first. */
export function latestTodoList(messages: ReadonlyArray<HistoryMessage>): Todo[] | undefined {
  for (const message of messages.toReversed()) {
    if (message.type !== "assistant") continue

    for (const part of message.content.toReversed()) {
      if (part.type !== "tool" || part.name !== TODO_TOOL || part.state.status !== "completed") continue

      const todos = readTodoInput(part.state.input)

      if (todos !== undefined) return todos
    }
  }

  return undefined
}
