import type { ToolEditor } from "@opencode/plugin/effect/tool"
import { Effect, type Scope } from "effect"
import { Host } from "./host.js"
import { type HostTexts, type Texts, withTexts } from "./texts.js"

/** OpenCode's tool that starts a child session. */
const SUBAGENT_TOOL = "subagent"

/**
 * Chauffeur's edits to the host's tool descriptions, in one transform so a
 * re-registration never reorders them: each `tool_descriptions` text replaces
 * that tool's description, then the user's standing instruction about
 * subagent models is appended to `subagent`, so the agent reads it before
 * launching one. Worded as the user's instruction: the host's own text says
 * to set a subagent's model only when the user asks.
 */
export const installSubagentGuidance: Effect.Effect<void, never, Host | Texts | Scope.Scope> = Effect.gen(function* () {
  const host = yield* Host

  yield* withTexts((texts) => host.tool.transform((editor) => {
    for (const [name, description] of Object.entries(texts.tool_descriptions)) {
      editor.update(name, (tool) => {
        tool.description = description
      })
    }

    appendGuidance(editor, texts)
  }))
})

function appendGuidance(editor: ToolEditor, texts: HostTexts): void {
  const guidance = texts.subagent.guidance

  editor.update(SUBAGENT_TOOL, (tool) => {
    if (!tool.description.includes(guidance)) tool.description = `${tool.description}\n\n${guidance}`
  })
}
