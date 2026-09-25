import { type Plugin, Skill } from "@opencode/plugin/effect"
import type { SessionPrompt } from "@opencode/plugin/effect/session"
import { Effect, Schema, type Scope } from "effect"
import { codeModeNamespaces } from "./code-mode.js"
import { Daemon } from "./daemon.js"
import { type HistoryMessage, Host, type HostTool, type SessionID } from "./host.js"
import { hostModel, ref, sameModel } from "./model-router.js"
import { ASK_TOOL } from "./ask-tool.js"
import { catalogEntry, signal, TEXT_CODE_POINTS, type CatalogEntry, type CodeModeNamespace, type ContextEffect, type HostEffect, type ModelRef } from "./protocol.js"
import { SKILLS_METADATA_KEY } from "./skills.js"
import { clip, isIntegrationMessage } from "./text.js"

const MAX_SESSIONS = 256

const MAX_AGENTS = 64

const MAX_CATALOG = 128

// Admission waits for exposure; past this the prompt proceeds unenhanced.
const EXPOSURE_TIMEOUT = "6 seconds"

const HIDDEN_METADATA_KEY = "chauffeur.hidden"

const isStringList = Schema.is(Schema.Array(Schema.String))

const storageKey = (sessionID: string): string => `hidden/${sessionID}`

/** Tools hidden for the context; `null` records that nothing is hidden. */
type Hidden = ReadonlySet<string> | null

type ToolsEffect = Extract<HostEffect, { readonly type: "tools" }>

export type ExposureControl = {
  readonly candidates: (sessionID: SessionID) => Effect.Effect<CatalogEntry[]>
  readonly reveal: (sessionID: SessionID, names: ReadonlyArray<string>) => Effect.Effect<boolean, unknown>
  /** `agent`'s Code Mode namespaces, each with its best matches for `text`. */
  readonly codeMode: (agent: string, text: string) => Effect.Effect<CodeModeNamespace[]>
}

/**
 * The tool names each agent's model requests carry, seen before anything is
 * removed. A session's first prompt precedes its first request, so tools are
 * kept per agent. The host names no default agent: a prompt shows whether its
 * session runs on the default, and that session's next request names it.
 */
class RequestTools {
  private readonly byAgent = new Map<string, ReadonlySet<string>>()
  private readonly onDefault = new Set<string>()
  private defaultTools: ReadonlySet<string> | null = null

  /** A prompt's session runs `agent`, or the host's default when it has none. */
  admit(sessionID: string, agent: string | undefined): void {
    if (agent !== undefined) {
      this.onDefault.delete(sessionID)

      return
    }

    if (this.onDefault.size >= MAX_SESSIONS) this.onDefault.clear()

    this.onDefault.add(sessionID)
  }

  record(sessionID: string, agent: string, tools: ReadonlySet<string>): void {
    if (!this.byAgent.has(agent) && this.byAgent.size >= MAX_AGENTS) this.byAgent.clear()

    this.byAgent.set(agent, tools)

    if (this.onDefault.has(sessionID)) this.defaultTools = tools
  }

  forAgent(agent: string | undefined): ReadonlySet<string> | null {
    return agent === undefined ? this.defaultTools : this.byAgent.get(agent) ?? null
  }
}

/** Each session's hidden tools, cached from storage or session history. */
class HiddenTools {
  private readonly sessions = new Map<string, Hidden>()
  readonly requests = new RequestTools()

  constructor(private readonly host: Plugin.Context) {}

  remember(sessionID: string, set: Hidden): void {
    if (!this.sessions.has(sessionID) && this.sessions.size >= MAX_SESSIONS) this.sessions.clear()

    this.sessions.set(sessionID, set)
  }

  cached(sessionID: string): Hidden {
    return this.sessions.get(sessionID) ?? null
  }

  current(sessionID: SessionID): Effect.Effect<Hidden> {
    const cached = this.sessions.get(String(sessionID))

    if (cached !== undefined) return Effect.succeed(cached)

    return stored(this.host, sessionID).pipe(Effect.tap((set) => Effect.sync(() => this.remember(String(sessionID), set))))
  }
}

/**
 * Senses user prompts with the skill and tool catalogs, attaches the skills
 * the engine selects to the prompt, and hides the tools the engine judged
 * unneeded for the context, bringing them back when the engine says so.
 * Everything is derived from session history, so a restart loses nothing.
 */
export const installExposure: Effect.Effect<ExposureControl, never, Host | Daemon | Scope.Scope> = Effect.gen(function* () {
  const host = yield* Host
  const daemon = yield* Daemon
  const hidden = new HiddenTools(host)

  yield* host.session.hook("context", (event) => Effect.gen(function* () {
    hidden.requests.record(String(event.sessionID), String(event.agent), new Set(Object.keys(event.tools)))

    const set = yield* hidden.current(event.sessionID)

    if (set === null) return

    // Only named tools are removed, so a tool the engine never judged stays.
    for (const name of set) delete event.tools[name]
  }))

  yield* host.session.hook("prompt", (event) => {
    // A sourcefed event, not the user: it reaches the engine from sourcefed.
    if (isIntegrationMessage(event.metadata)) return Effect.void

    const sessionID = String(event.sessionID)

    return Effect.gen(function* () {
      const { firstInContext, skills, model, agent } = yield* admission(host, event, (set) => hidden.remember(sessionID, set))
      const requestTools = hidden.requests.forAgent(agent)

      hidden.requests.admit(sessionID, agent)

      // A new context hides nothing until the engine decides; a later message
      // offers the hidden tools, which the engine may bring back.
      if (firstInContext) {
        yield* host.storage.set(storageKey(sessionID), [])
        hidden.remember(sessionID, null)
      }

      const current = firstInContext ? null : yield* hidden.current(event.sessionID)
      const tools = yield* toolsToJudge(host, firstInContext, current, requestTools)
      const codeMode = codeModeNamespaces(yield* host.tool.list(), requestTools, event.prompt.text)

      const effects = yield* daemon.signal(signal(sessionID, {
        type: "user_message",
        text: clip(event.prompt.text, TEXT_CODE_POINTS),
        first_in_context: firstInContext,
        skills: skills.map((skill) => catalogEntry(skill.id, skill.description ?? skill.name, skill.content)),
        tools: tools.map((tool) => catalogEntry(tool.id, tool.description, "")),
        model: model ? ref(model) : null,
        code_mode: codeMode,
      }), EXPOSURE_TIMEOUT)

      yield* apply(host, event, effects, {
        hidden: hidden.cached(sessionID),
        expected: model ? ref(model) : null,
        remember: (set) => hidden.remember(sessionID, set),
      })
    }).pipe(
      // Exposure fails open: the prompt is admitted unenhanced.
      Effect.catch((error) => Effect.logError("chauffeur: exposure unavailable", error)),
    )
  })

  return control(host, hidden)
})

/** Hidden tools other capabilities may offer the engine, and reveal once it confirms them. */
function control(host: Plugin.Context, hidden: HiddenTools): ExposureControl {
  return {
    candidates: (sessionID) => Effect.gen(function* () {
      const set = yield* hidden.current(sessionID)

      if (set === null) return []

      return (yield* host.tool.list())
        .filter((tool) => set.has(tool.id) && tool.id !== "skill" && tool.options?.codemode !== true)
        .slice(0, MAX_CATALOG)
        .map((tool) => catalogEntry(tool.id, tool.description, ""))
    }),
    codeMode: (agent, text) => host.tool.list().pipe(Effect.map((tools) => codeModeNamespaces(tools, hidden.requests.forAgent(agent), text))),
    reveal: (sessionID, names) => Effect.gen(function* () {
      const set = yield* hidden.current(sessionID)

      if (set === null || names.length === 0) return false

      const registered = new Set((yield* host.tool.list()).map((tool) => tool.id))

      if (!names.every((name) => set.has(name) && registered.has(name))) return false

      const remaining = [...set].filter((name) => !names.includes(name))

      yield* host.session.synthetic({
        sessionID,
        text: " ",
        description: "Chauffeur tool exposure",
        metadata: { [HIDDEN_METADATA_KEY]: remaining },
        delivery: "steer",
        resume: false,
      })
      yield* host.storage.set(storageKey(String(sessionID)), remaining)
      hidden.remember(String(sessionID), remaining.length > 0 ? new Set(remaining) : null)

      return true
    }),
  }
}

/** What applying the engine's effects to one prompt needs. */
type Applying = { hidden: Hidden; expected: ModelRef | null; remember: (set: Hidden) => void }

function apply(host: Plugin.Context, event: SessionPrompt, effects: ReadonlyArray<HostEffect>, applying: Applying): Effect.Effect<void, unknown> {
  return Effect.forEach(effects, (effect) => {
    if (effect.agent_id !== String(event.sessionID)) return Effect.void

    if (effect.type === "model" && effect.model) return switchModel(host, event, effect.model, applying.expected)

    if (effect.type === "tools") return hideTools(host, event, effect, applying)

    if (effect.type === "context" && effect.delivery === "prompt") attach(effect, event)

    return Effect.void
  }, { discard: true })
}

/** Switch the session's model, unless the selection changed while the engine judged. */
function switchModel(host: Plugin.Context, event: SessionPrompt, next: ModelRef, expected: ModelRef | null): Effect.Effect<void, unknown> {
  return Effect.gen(function* () {
    const current = (yield* host.session.get({ sessionID: event.sessionID })).model

    if (!current || !expected || !sameModel(ref(current), expected)) return

    yield* host.session.switchModel({ sessionID: event.sessionID, model: hostModel(next) })
  })
}

/** Store the context's hidden tools and record them on this message, so a restart restores them. */
function hideTools(host: Plugin.Context, event: SessionPrompt, effect: ToolsEffect, applying: Applying): Effect.Effect<void> {
  const hide = hiddenAfter(effect, applying.hidden)

  applying.remember(hide.length > 0 ? new Set(hide) : null)
  event.metadata = { ...event.metadata, [HIDDEN_METADATA_KEY]: hide }

  return host.storage.set(storageKey(String(event.sessionID)), hide)
}

/**
 * The prompt's context, whether it starts one, the skills to judge, and the
 * session's model. A subagent's first prompt continues its parent's context
 * instead of starting one.
 */
function admission(host: Plugin.Context, event: SessionPrompt, remember: (set: Hidden) => void) {
  return Effect.gen(function* () {
    const context = currentContext(yield* host.session.context({ sessionID: event.sessionID }))
    const fresh = !context.some((message) => message.type === "user" && !isIntegrationMessage(message.metadata))
    const session = yield* host.session.get({ sessionID: event.sessionID })

    if (fresh && session.parentID) yield* inherit(host, event, session.parentID, remember)

    return {
      firstInContext: fresh && !session.parentID,
      skills: yield* skillsToJudge(host, context, event),
      model: session.model,
      agent: session.agent,
    }
  })
}

/**
 * Start a subagent from its parent: the parent's skills are attached to its
 * first prompt, and the parent's hidden tools stay hidden, so the engine
 * judges only what the subagent adds.
 */
function inherit(host: Plugin.Context, event: SessionPrompt, parentID: SessionID, remember: (set: Hidden) => void): Effect.Effect<void, unknown> {
  return Effect.gen(function* () {
    const parent = currentContext(yield* host.session.context({ sessionID: parentID }))
    const present = new Set<string>((event.prompt.skills ?? []).map((skill) => skill.id))
    const skills = [...attachedSkills(parent)].flatMap((id) => (present.has(id) ? [] : [{ id: Skill.ID.make(id) }]))
    const set = yield* stored(host, parentID)

    event.prompt.skills = [...(event.prompt.skills ?? []), ...skills]
    remember(set)
    yield* host.storage.set(storageKey(String(event.sessionID)), [...(set ?? [])])

    if (set !== null) event.metadata = { ...event.metadata, [HIDDEN_METADATA_KEY]: [...set] }
  })
}

/** A session's hidden tools from storage, or from its history before storage held them. */
function stored(host: Plugin.Context, sessionID: SessionID): Effect.Effect<Hidden> {
  return host.storage.get(storageKey(String(sessionID))).pipe(
    Effect.flatMap((value) => isStringList(value)
      ? Effect.succeed(value.length > 0 ? new Set(value) : null)
      : restoreHidden(host, sessionID)),
    // Fails open: an unreadable record hides nothing.
    Effect.orElseSucceed(() => null),
  )
}

/** Skills not yet in this context, within the catalog cap. */
function skillsToJudge(host: Plugin.Context, context: ReadonlyArray<HistoryMessage>, event: SessionPrompt) {
  const attached = attachedSkills(context, event)

  return host.skill.list().pipe(Effect.map((skills) => skills.data.filter((skill) => !attached.has(skill.id)).slice(0, MAX_CATALOG)))
}

/** Skills already in this context: attached to user messages or delivered mid-turn. */
function attachedSkills(context: ReadonlyArray<HistoryMessage>, event?: SessionPrompt): Set<string> {
  return new Set([
    ...context.flatMap((message) => (message.type === "user" ? message.skills ?? [] : [])).map((skill) => skill.id),
    ...context.flatMap((message) => {
      const ids = message.type === "synthetic" ? message.metadata?.[SKILLS_METADATA_KEY] : undefined

      return isStringList(ids) ? ids : []
    }),
    ...(event?.prompt.skills ?? []).map((skill) => skill.id),
  ])
}

/**
 * Skills join the prompt as user-selected skills; text joins it as a text
 * attachment. Both reach the first step and add nothing to the system prompt.
 */
function attach(effect: ContextEffect, event: SessionPrompt): void {
  event.prompt.skills = [...(event.prompt.skills ?? []), ...effect.skills.map((id) => ({ id: Skill.ID.make(id) }))]

  if (effect.text) {
    const name = `chauffeur-${effect.label.toLowerCase().replace(/[^a-z0-9]+/g, "-")}.txt`

    event.prompt.files = [...(event.prompt.files ?? []), { uri: `data:text/plain;base64,${Buffer.from(effect.text, "utf8").toString("base64")}`, name }]
  }
}

/** The context's hidden tools after applying the engine's decision. */
function hiddenAfter(effect: ToolsEffect, hidden: Hidden): string[] {
  return [...new Set([...(hidden ?? []), ...effect.hide])].filter((tool) => !effect.reveal.includes(tool))
}

/** Messages since the most recent compaction, which begins a new context. */
function currentContext(messages: ReadonlyArray<HistoryMessage>): HistoryMessage[] {
  const lastCompaction = messages.findLastIndex((message) => message.type === "compaction")

  return messages.slice(lastCompaction + 1)
}

function restoreHidden(host: Plugin.Context, sessionID: SessionID): Effect.Effect<Hidden, unknown> {
  return host.session.context({ sessionID }).pipe(Effect.map((messages) => {
    const recorded = currentContext(messages)
      .flatMap((message) => (message.type === "user" || message.type === "synthetic" ? [message.metadata?.[HIDDEN_METADATA_KEY]] : []))
      .findLast((value) => Array.isArray(value))

    return isStringList(recorded) ? new Set(recorded) : null
  }))
}

/** A new context's request tools, or a later message's hidden tools. */
function toolsToJudge(host: Plugin.Context, firstInContext: boolean, hidden: Hidden, requestTools: ReadonlySet<string> | null): Effect.Effect<HostTool[]> {
  return host.tool.list().pipe(Effect.map((tools) => {
    if (firstInContext) return catalog(tools, requestTools)

    if (hidden === null) return []

    return catalog(tools.filter((tool) => hidden.has(tool.id)), null)
  }))
}

/**
 * The tools worth judging, within the catalog cap: `skill` first (it is always
 * hidden), then the other tools the host sends in model requests. Code Mode
 * tools reach the model through `execute`, not the request's tool record, so
 * hiding them would change nothing. Before any request is seen, only tools
 * flagged as Code Mode are left out. Chauffeur's own tool is never judged, so
 * the agent can always ask for what it lacks.
 */
function catalog(tools: ReadonlyArray<HostTool>, inRequests: ReadonlySet<string> | null): HostTool[] {
  return tools
    .filter((tool) => tool.id !== ASK_TOOL)
    .filter((tool) => (inRequests ? inRequests.has(tool.id) : tool.options?.codemode !== true))
    .toSorted((a, b) => Number(b.id === "skill") - Number(a.id === "skill"))
    .slice(0, MAX_CATALOG)
}
