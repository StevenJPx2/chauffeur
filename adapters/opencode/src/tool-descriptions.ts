import type { SessionContext } from "@opencode/plugin/effect/session"
import { Effect, type Scope } from "effect"
import { Host } from "./host.js"
import { type HostTexts, Texts } from "./texts.js"

/** OpenCode's tool that starts a child session. */
const SUBAGENT_TOOL = "subagent"

/**
 * Chauffeur's edits to the tool descriptions each request carries, applied
 * where every tool is final (Code Mode's `execute` and per-request texts
 * included): each `tool_descriptions` text replaces that tool's description,
 * then the user's standing instruction about subagent models is appended to
 * `subagent`, so the agent reads it before launching one. Worded as the
 * user's instruction: the host's own text says to set a subagent's model
 * only when the user asks.
 */
export const installToolDescriptions: Effect.Effect<void, never, Host | Texts | Scope.Scope> = Effect.gen(function* () {
  const host = yield* Host
  const texts = yield* Texts

  yield* host.session.hook("context", (event) => Effect.sync(() => rewriteDescriptions(event.tools, texts.current())))
})

/** Rewrite `tools`' descriptions in place from `texts`. */
export function rewriteDescriptions(tools: SessionContext["tools"], texts: HostTexts): void {
  for (const [name, description] of Object.entries(texts.tool_descriptions)) {
    const tool = tools[name]

    if (tool) tool.description = description
  }

  const subagent = tools[SUBAGENT_TOOL]
  const guidance = texts.subagent.guidance

  if (subagent && !subagent.description.includes(guidance)) subagent.description = `${subagent.description}\n\n${guidance}`
}
