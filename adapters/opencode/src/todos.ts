import { Effect, Option, Schema } from "effect"
import { type HistoryMessage, Host } from "./host.js"
import { type Todo, TodosSchema } from "./todo-data.js"
import { latestTodoList } from "./transcript-todos.js"

const storageKey = (sessionID: string): string => `todos/${sessionID}`

/**
 * The session's todo list, which the agent writes with a `todowrite` tool of
 * some plugin's: the last write in its transcript. A transcript can lose it, to
 * a compaction, so the last list seen is kept and read back then; empty when
 * there has never been one.
 */
export function readTodos(sessionID: string, messages: ReadonlyArray<HistoryMessage>): Effect.Effect<ReadonlyArray<Todo>, never, Host> {
  return Effect.gen(function* () {
    const host = yield* Host
    const written = latestTodoList(messages)

    if (written !== undefined) {
      yield* host.storage.set(storageKey(sessionID), written)

      return written
    }

    const stored = yield* host.storage.get(storageKey(sessionID))

    return Option.getOrElse(Schema.decodeUnknownOption(TodosSchema)(stored), (): ReadonlyArray<Todo> => [])
  })
}
