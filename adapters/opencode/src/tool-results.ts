import type { Plugin } from "@opencode/plugin/effect"
import type { ToolHooks } from "@opencode/plugin/effect/tool"
import { Effect, type Scope } from "effect"
import { Daemon } from "./daemon.js"
import type { ExposureControl } from "./exposure.js"
import { Host } from "./host.js"
import { signal, TEXT_CODE_POINTS, type CatalogEntry, type HostEffect } from "./protocol.js"
import { deliverContext } from "./skills.js"
import { clip } from "./text.js"

const ERROR_CODE_POINTS = 240

const TOOL_RESULT_TIMEOUT = "3 seconds"

const RECOVERY_TIMEOUT = "8 seconds"

const MAX_CANDIDATES = 4

const MISSING_TOOL = /(?:unknown|missing|unavailable|not found|no matching|not registered|cannot call|not available).{0,50}(?:tool|function)|(?:tool|function).{0,50}(?:unknown|missing|unavailable|not found|not registered|not available)/i

type ToolResult = ToolHooks["execute.after"]

/**
 * Report tool results, deliver mid-turn context into the running turn, and
 * apply verified hidden-tool reveals.
 */
export function installToolResults(exposure: ExposureControl): Effect.Effect<void, never, Host | Daemon | Scope.Scope> {
  return Effect.gen(function* () {
    const host = yield* Host
    const daemon = yield* Daemon

    yield* host.tool.hook("execute.after", (event) =>
      report(event, exposure).pipe(
        Effect.provideService(Host, host),
        Effect.provideService(Daemon, daemon),
        Effect.catch((error) => Effect.logError("chauffeur: tool result not reported", error)),
      ))
  })
}

function report(event: ToolResult, exposure: ExposureControl): Effect.Effect<void, unknown, Host | Daemon> {
  return Effect.gen(function* () {
    const host = yield* Host
    const daemon = yield* Daemon
    const observed = observe(event)
    const recovery = MISSING_TOOL.test(observed.evidence) ? yield* recover(host, exposure, event, observed) : NO_RECOVERY

    // An unreadable session still reports the result, without a workspace.
    const workspace = yield* host.session.get({ sessionID: event.sessionID }).pipe(
      Effect.map((session) => clip(String(session.location.directory), TEXT_CODE_POINTS)),
      Effect.orElseSucceed(() => ""),
    )

    const effects = yield* daemon.signal(signal(String(event.sessionID), {
      type: "tool_result",
      tool: clip(event.tool, TEXT_CODE_POINTS),
      ok: event.status === "completed",
      workspace,
      ...observed,
      user_request: recovery.userRequest,
      candidates: recovery.candidates,
    }), recovery.candidates.length > 0 ? RECOVERY_TIMEOUT : TOOL_RESULT_TIMEOUT)

    yield* applyEffects(event, effects, exposure)
  })
}

type Observed = { readonly input: string; readonly error: string; readonly evidence: string }

type Recovery = { readonly userRequest: string; readonly candidates: CatalogEntry[] }

const NO_RECOVERY: Recovery = { userRequest: "", candidates: [] }

/** The call's input, its error, and the bounded evidence both leave. */
function observe(event: ToolResult): Observed {
  const input = summarize("input", () => JSON.stringify(event.input))
  const error = event.status === "error" ? clip(event.error.message, ERROR_CODE_POINTS) : ""
  const output = event.status === "completed" ? summarize("output", () => JSON.stringify(event.result.content ?? event.result.output)) : ""

  return { input, error, evidence: clip(`${error} ${output}`.trim(), TEXT_CODE_POINTS) }
}

/** For a missing tool: the user's latest request and the hidden tools it could mean. */
function recover(host: Plugin.Context, exposure: ExposureControl, event: ToolResult, observed: Observed): Effect.Effect<Recovery, unknown> {
  return Effect.gen(function* () {
    const lastUser = (yield* host.session.context({ sessionID: event.sessionID })).findLast((message) => message.type === "user")
    const userRequest = lastUser?.type === "user" ? clip(lastUser.text, TEXT_CODE_POINTS) : ""
    const catalog = yield* exposure.candidates(event.sessionID)

    return { userRequest, candidates: matching(catalog, `${observed.input} ${observed.evidence} ${userRequest}`) }
  })
}

/** Reveal confirmed tools; deliver context into this turn, each skill once. */
function applyEffects(event: ToolResult, effects: ReadonlyArray<HostEffect>, exposure: ExposureControl): Effect.Effect<void, unknown, Host> {
  const delivered = new Set<string>()

  return Effect.forEach(effects, (effect) => {
    if (effect.agent_id !== String(event.sessionID)) return Effect.void

    if (effect.type === "tools") return effect.reveal.length > 0 ? exposure.reveal(event.sessionID, effect.reveal) : Effect.void

    // Prompt context joins a user message in the prompt hook; the rest reaches this turn.
    if (effect.type !== "context" || effect.delivery === "prompt") return Effect.void

    const skills = effect.skills.filter((skill) => !delivered.has(skill))

    for (const skill of skills) delivered.add(skill)

    return deliverContext(event.sessionID, { ...effect, skills })
  }, { discard: true })
}

/** Lexical matches narrow the registered hidden catalog; Jev judges need. */
function matching(catalog: ReadonlyArray<CatalogEntry>, text: string): CatalogEntry[] {
  const words = new Set(text.toLowerCase().match(/[a-z][a-z0-9]{3,}/g) ?? [])

  return catalog.filter((tool) => {
    const names = tool.id.toLowerCase().split(/[_-]/)

    return names.some((name) => name.length >= 4 && words.has(name))
      || tool.description.toLowerCase().split(/\W+/).filter((word) => word.length >= 5 && words.has(word)).length >= 2
  }).slice(0, MAX_CANDIDATES)
}

function summarize(part: "input" | "output", encode: () => string | undefined): string {
  try {
    return clip(encode() ?? "", TEXT_CODE_POINTS)
  } catch {
    return `unserializable ${part}`
  }
}
