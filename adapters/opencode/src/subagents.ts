import { Effect, type Scope } from "effect"
import { Host } from "./host.js"
import { type Texts, withTexts } from "./texts.js"

/** OpenCode's tool that starts a child session. */
const SUBAGENT_TOOL = "subagent"

/**
 * Append the user's standing instruction about subagent models to the
 * host's `subagent` description, so the agent reads it before launching one.
 * Worded as the user's instruction: the host's own text says to set a
 * subagent's model only when the user asks.
 */
export const installSubagentGuidance: Effect.Effect<void, never, Host | Texts | Scope.Scope> = Effect.gen(function* () {
  const host = yield* Host

  yield* withTexts((texts) => host.tool.transform((editor) => {
    const guidance = texts.subagent.guidance

    editor.update(SUBAGENT_TOOL, (tool) => {
      if (!tool.description.includes(guidance)) tool.description = `${tool.description}\n\n${guidance}`
    })
  }))
})
