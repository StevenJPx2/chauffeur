import type { ToolEditor } from "@opencode/plugin/effect/tool"
import { Effect, Schema, type Scope } from "effect"
import { Daemon } from "./daemon.js"
import type { ExposureControl } from "./exposure.js"
import { Host, type SessionID } from "./host.js"
import { signal, TEXT_CODE_POINTS, type HostEffect } from "./protocol.js"
import { hostLoadsSkills, renderContext } from "./skills.js"
import { clip, isIntegrationMessage, userText } from "./text.js"
import { applyNamespaces } from "./namespaces.js"
import { fill, type HostTexts, type Texts, withTexts } from "./texts.js"

/** Chauffeur's own tool; exposure never judges or hides it. */
export const ASK_TOOL = "ask_chauffeur"

// One Jev call judges the tools and skills together.
const ASK_TIMEOUT = "8 seconds"

/** The reply when nothing was granted; the host's skill tool is named while the host keeps it. */
export function nothingFound(texts: HostTexts, hostSkills: boolean): string {
  return hostSkills ? texts.ask_chauffeur.nothing_found_skill_tool : texts.ask_chauffeur.nothing_found
}

/** The host decodes the call's input with this schema before `execute` sees it. */
function inputSchema(texts: HostTexts) {
  return Schema.Struct({
    need: Schema.String.annotate({ description: texts.ask_chauffeur.need }),
  })
}

type Input = { readonly need: string }

/** What the host tells a tool about the call. */
type CallContext = Parameters<Parameters<ToolEditor["add"]>[0]["execute"]>[1]

/**
 * Register `ask_chauffeur`: the agent says what it lacks, the engine picks
 * the hidden tools, Code Mode tools, or skill that serve it, and the reply
 * carries what it granted. Revealed tools join the agent's next step.
 */
export function installAskTool(exposure: ExposureControl): Effect.Effect<void, never, Host | Daemon | Texts | Scope.Scope> {
  return Effect.gen(function* () {
    const host = yield* Host
    const daemon = yield* Daemon

    yield* withTexts((texts) => host.tool.transform((editor) => {
      editor.add({
        name: ASK_TOOL,
        description: texts.ask_chauffeur.description,
        input: inputSchema(texts),
        options: { codemode: false },
        execute: (input, context) =>
          answer(input, context, exposure, texts).pipe(
            Effect.provideService(Host, host),
            Effect.provideService(Daemon, daemon),
            Effect.catch((error) => Effect.succeed(fill(texts.ask_chauffeur.failed, { error: String(error) }))),
            Effect.map((content) => ({ content })),
          ),
      })
    }))
  })
}

function answer({ need }: Input, context: CallContext, exposure: ExposureControl, texts: HostTexts): Effect.Effect<string, unknown, Host | Daemon> {
  return Effect.gen(function* () {
    const host = yield* Host
    const daemon = yield* Daemon
    const history = yield* host.session.context({ sessionID: context.sessionID })
    const lastUser = history.findLast((message) => message.type === "user" && !isIntegrationMessage(message.metadata))
    const clipped = clip(need, TEXT_CODE_POINTS)

    const effects = yield* daemon.signal(signal(String(context.sessionID), {
      type: "agent_request",
      need: clipped,
      user_request: lastUser?.type === "user" ? userText(lastUser.text) : "",
      tools: yield* exposure.candidates(context.sessionID),
      code_mode: yield* exposure.codeMode(clipped),
    }), ASK_TIMEOUT)

    const parts = yield* Effect.forEach(effects, (effect) => granted(context.sessionID, effect, exposure, texts))
    const reply = parts.filter((part) => part !== "").join("\n\n")

    return reply === "" ? nothingFound(texts, hostLoadsSkills()) : reply
  })
}

/** What one effect grants the agent, as reply text. */
function granted(sessionID: SessionID, effect: HostEffect, exposure: ExposureControl, texts: HostTexts): Effect.Effect<string, unknown, Host> {
  return Effect.gen(function* () {
    if (effect.agent_id !== String(sessionID)) return ""

    if (effect.type === "tools") {
      if (effect.reveal.length === 0 || !(yield* exposure.reveal(sessionID, effect.reveal))) return ""

      return fill(texts.ask_chauffeur.revealed, { tools: effect.reveal.join(", ") })
    }

    // Restored namespaces join the catalog at the next request; the Code Mode
    // note in this reply names their tools.
    if (effect.type === "namespaces") {
      yield* applyNamespaces(sessionID, effect)

      return ""
    }

    if (effect.type !== "context") return ""

    return (yield* renderContext(effect)).text
  })
}
