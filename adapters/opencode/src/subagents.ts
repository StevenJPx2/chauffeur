import { Effect, type Scope } from "effect"
import { Host } from "./host.js"

/** OpenCode's tool that starts a child session. */
const SUBAGENT_TOOL = "subagent"

/**
 * The user's standing instruction, so the host's own "only when the user
 * asks" rule on `model` does not hold the agent back.
 */
export const SUBAGENT_GUIDANCE = [
  "The user asks that subagents doing routine work, such as searching, reading and summarising, or mechanical edits, run on a cheaper model from your own provider:",
  "pass model \"anthropic/claude-sonnet-5-5\" when you run on Claude Opus, or \"openai/gpt-6-luna\" when you run on GPT-6.1 Sol.",
  "Keep your own model for hard design, debugging or review.",
].join(" ")

/** Tell the agent, before it launches one, which model a routine subagent should use. */
export const installSubagentGuidance: Effect.Effect<void, never, Host | Scope.Scope> = Effect.gen(function* () {
  const host = yield* Host

  yield* host.tool.transform((editor) => {
    editor.update(SUBAGENT_TOOL, (tool) => {
      if (!tool.description.includes(SUBAGENT_GUIDANCE)) tool.description = `${tool.description}\n\n${SUBAGENT_GUIDANCE}`
    })
  })
})
