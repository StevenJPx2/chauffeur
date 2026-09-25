import type { ToolEditor } from "@opencode/plugin/effect/tool"
import { Effect, Schema, type Scope } from "effect"
import { Daemon } from "./daemon.js"
import type { ExposureControl } from "./exposure.js"
import { Host, type SessionID } from "./host.js"
import { signal, TEXT_CODE_POINTS, type HostEffect } from "./protocol.js"
import { renderContext } from "./skills.js"
import { clip, isIntegrationMessage } from "./text.js"

/** Chauffeur's own tool; exposure never judges or hides it. */
export const ASK_TOOL = "ask_chauffeur"

// One Jev call judges the tools and skills together.
const ASK_TIMEOUT = "8 seconds"

const NOTHING = "Chauffeur found no hidden tool, Code Mode tool, or skill for that. Continue with the tools you have."

/** The host decodes the call's input with this schema before `execute` sees it. */
const Input = Schema.Struct({
  need: Schema.String.annotate({ description: "What you need to do, in plain words." }),
})

type Input = typeof Input.Type

/** What the host tells a tool about the call. */
type CallContext = Parameters<Parameters<ToolEditor["add"]>[0]["execute"]>[1]

const DESCRIPTION = [
  "Ask Chauffeur for a tool or skill you lack.",
  "Describe what you need to do in plain words, such as \"drive a browser to check a page\" or \"create a Jira issue from the command line\".",
  "Chauffeur may reveal hidden tools, point you to Code Mode tools, or hand over a skill; the reply says what you got.",
  "Ask before improvising a workaround.",
].join(" ")

/**
 * Register `ask_chauffeur`: the agent says what it lacks, the engine picks
 * the hidden tools, Code Mode tools, or skill that serve it, and the reply
 * carries what it granted. Revealed tools join the agent's next step.
 */
export function installAskTool(exposure: ExposureControl): Effect.Effect<void, never, Host | Daemon | Scope.Scope> {
  return Effect.gen(function* () {
    const host = yield* Host
    const daemon = yield* Daemon

    yield* host.tool.transform((editor) => {
      editor.add({
        name: ASK_TOOL,
        description: DESCRIPTION,
        input: Input,
        options: { codemode: false },
        execute: (input, context) =>
          answer(input, context, exposure).pipe(
            Effect.provideService(Host, host),
            Effect.provideService(Daemon, daemon),
            Effect.catch((error) => Effect.succeed(`Chauffeur could not answer: ${String(error)}. Continue with the tools you have.`)),
            Effect.map((content) => ({ content })),
          ),
      })
    })
  })
}

function answer({ need }: Input, context: CallContext, exposure: ExposureControl): Effect.Effect<string, unknown, Host | Daemon> {
  return Effect.gen(function* () {
    const host = yield* Host
    const daemon = yield* Daemon
    const history = yield* host.session.context({ sessionID: context.sessionID })
    const lastUser = history.findLast((message) => message.type === "user" && !isIntegrationMessage(message.metadata))
    const clipped = clip(need, TEXT_CODE_POINTS)

    const effects = yield* daemon.signal(signal(String(context.sessionID), {
      type: "agent_request",
      need: clipped,
      user_request: lastUser?.type === "user" ? clip(lastUser.text, TEXT_CODE_POINTS) : "",
      tools: yield* exposure.candidates(context.sessionID),
      code_mode: yield* exposure.codeMode(String(context.agent), clipped),
    }), ASK_TIMEOUT)

    const parts = yield* Effect.forEach(effects, (effect) => granted(context.sessionID, effect, exposure))
    const reply = parts.filter((part) => part !== "").join("\n\n")

    return reply === "" ? NOTHING : reply
  })
}

/** What one effect grants the agent, as reply text. */
function granted(sessionID: SessionID, effect: HostEffect, exposure: ExposureControl): Effect.Effect<string, unknown, Host> {
  return Effect.gen(function* () {
    if (effect.agent_id !== String(sessionID)) return ""

    if (effect.type === "tools") {
      if (effect.reveal.length === 0 || !(yield* exposure.reveal(sessionID, effect.reveal))) return ""

      return `Chauffeur revealed these tools; they are available from your next step: ${effect.reveal.join(", ")}.`
    }

    if (effect.type !== "context") return ""

    return (yield* renderContext(effect)).text
  })
}
