import type { Plugin, Skill } from "@opencode/plugin"
import type { SessionPrompt } from "@opencode/plugin/promise/session"
import type { DaemonBridge } from "./daemon.js"
import { hostModel, ref, sameModel, signal } from "./model-router.js"
import type { CatalogEntry, ContextEffect, Effect, ModelRef } from "./protocol.js"
import { codeModeNamespaces } from "./code-mode.js"
import { SKILLS_METADATA_KEY } from "./skills.js"
import { clip, isIntegrationMessage } from "./text.js"

const MAX_SESSIONS = 256
const MAX_CATALOG = 128
const TEXT_CODE_POINTS = 512
// 100 code points stays within the engine's 400-byte description bound.
const DESCRIPTION_CODE_POINTS = 100
// Admission waits for exposure; past this the prompt proceeds unenhanced.
const EXPOSURE_TIMEOUT_MS = 6_000
const HIDDEN_METADATA_KEY = "chauffeur.hidden"
const storageKey = (sessionID: string): string => `hidden/${sessionID}`

type HistoryMessage = Awaited<ReturnType<Plugin.Context["session"]["context"]>>[number]
type SessionID = Parameters<Plugin.Context["session"]["context"]>[0]["sessionID"]
/** Tools hidden for the context; `null` records that nothing is hidden. */
type Hidden = ReadonlySet<string> | null
export type ExposureControl = {
  dispose: () => Promise<void>
  candidates: (sessionID: SessionID) => Promise<CatalogEntry[]>
  reveal: (sessionID: SessionID, names: ReadonlyArray<string>) => Promise<boolean>
}

/**
 * Senses user prompts with the skill and tool catalogs, attaches the skills
 * the engine selects to the prompt, and hides the tools the engine judged
 * unneeded for the context, bringing them back when the engine says so.
 * Everything is derived from session history, so a restart loses nothing.
 */
export async function installExposure(ctx: Plugin.Context, daemon: DaemonBridge): Promise<ExposureControl> {
  const hiddenTools = new Map<string, Hidden>()
  // Tool names the host puts in model requests, seen before anything is removed.
  let requestTools: ReadonlySet<string> | null = null
  const remember = (sessionID: string, set: Hidden): void => {
    if (!hiddenTools.has(sessionID) && hiddenTools.size >= MAX_SESSIONS) hiddenTools.clear()

    hiddenTools.set(sessionID, set)
  }
  const current = async (sessionID: string, id: SessionID): Promise<Hidden> => {
    let set = hiddenTools.get(sessionID)

    if (set === undefined) {
      const stored = await ctx.storage.get(storageKey(sessionID)).catch(() => undefined)
      set = isHiddenList(stored) ? (stored.length > 0 ? new Set(stored) : null) : await restoreHidden(ctx, id).catch(() => null)
      remember(sessionID, set)
    }

    return set
  }

  const context = await ctx.session.hook("context", async (event) => {
    requestTools = new Set(Object.keys(event.tools))

    const set = await current(String(event.sessionID), event.sessionID)

    if (set === null) return

    // Only named tools are removed, so a tool the engine never judged stays.
    for (const name of set) delete event.tools[name]
  })
  const prompt = await ctx.session.hook("prompt", async (event) => {
    const sessionID = String(event.sessionID)

    // A sourcefed event, not the user: it reaches the engine from sourcefed.
    if (isIntegrationMessage(event.metadata)) return

    try {
      const { context, firstInContext, skills, model } = await admission(ctx, event, (set) => remember(sessionID, set))

      // A new context hides nothing until the engine decides; a later message
      // offers the hidden tools, which the engine may bring back.
      if (firstInContext) {
        await ctx.storage.set(storageKey(sessionID), [])
        remember(sessionID, null)
      }

      const hidden = firstInContext ? null : await current(sessionID, event.sessionID)
      const tools = await toolsToJudge(ctx, firstInContext, hidden, requestTools)
      const codeMode = codeModeNamespaces(await ctx.tool.list(), requestTools, event.prompt.text)
      const effects = await daemon.signal(signal(sessionID, {
        type: "user_message",
        text: clip(event.prompt.text, TEXT_CODE_POINTS),
        first_in_context: firstInContext,
        skills: skills.map((skill) => entry(skill.id, skill.description ?? skill.name, skill.content)),
        tools: tools.map((tool) => entry(tool.id, tool.description, "")),
        model: model ? ref(model) : null,
        code_mode: codeMode,
      }), EXPOSURE_TIMEOUT_MS)

      await apply(ctx, event, effects, { hidden: hiddenTools.get(sessionID) ?? null, expected: model ? ref(model) : null, remember: (set) => remember(sessionID, set) })
    } catch (error) {
      // Exposure fails open: the prompt is admitted unenhanced.
      console.error(`[chauffeur] exposure unavailable: ${String(error)}`)
    }
  })

  return {
    dispose: async () => {
      await prompt.dispose()
      await context.dispose()
      hiddenTools.clear()
    },
    candidates: async (sessionID) => {
      const hidden = await current(String(sessionID), sessionID)
      if (hidden === null) return []
      const tools = await ctx.tool.list()
      return tools.filter((tool) => hidden.has(tool.id) && tool.id !== "skill" && tool.options?.codemode !== true)
        .slice(0, MAX_CATALOG).map((tool) => entry(tool.id, tool.description, ""))
    },
    reveal: async (sessionID, names) => {
      const hidden = await current(String(sessionID), sessionID)
      if (hidden === null || names.length === 0) return false
      const registered = new Set((await ctx.tool.list()).map((tool) => tool.id))
      if (!names.every((name) => hidden.has(name) && registered.has(name))) return false
      const remaining = [...hidden].filter((name) => !names.includes(name))
      await ctx.session.synthetic({
        sessionID, text: " ", description: "Chauffeur tool exposure",
        metadata: { [HIDDEN_METADATA_KEY]: remaining }, delivery: "steer", resume: false,
      })
      await ctx.storage.set(storageKey(String(sessionID)), remaining)
      remember(String(sessionID), remaining.length > 0 ? new Set(remaining) : null)
      return true
    },
  }
}

/** What applying the engine's effects to one prompt needs. */
type Applying = { hidden: Hidden; expected: ModelRef | null; remember: (set: Hidden) => void }

async function apply(ctx: Plugin.Context, event: SessionPrompt, effects: Effect[], applying: Applying): Promise<void> {
  for (const effect of effects) {
    if (effect.agent_id !== String(event.sessionID)) continue

    if (effect.type === "model" && effect.model) {
      const current = (await ctx.session.get({ sessionID: event.sessionID })).model

      if (!current || !applying.expected || !sameModel(ref(current), applying.expected)) continue
      await ctx.session.switchModel({ sessionID: event.sessionID, model: hostModel(effect.model) })
    } else if (effect.type === "tools") {
      await ctx.storage.set(storageKey(String(event.sessionID)), [...new Set([...(applying.hidden ?? []), ...effect.hide])].filter((tool) => !effect.reveal.includes(tool)))
      applyTools(effect, event, applying)
    } else if (effect.type === "context" && effect.delivery === "prompt") {
      attach(effect, event)
    }
  }
}

/**
 * The prompt's context, whether it starts one, the skills to judge, and the
 * session's model. A subagent's first prompt continues its parent's context
 * instead of starting one.
 */
async function admission(ctx: Plugin.Context, event: SessionPrompt, remember: (set: Hidden) => void) {
  const context = currentContext(await ctx.session.context({ sessionID: event.sessionID }))
  const fresh = !context.some((message) => message.type === "user" && !isIntegrationMessage(message.metadata))
  const session = await ctx.session.get({ sessionID: event.sessionID })

  if (fresh && session.parentID) await inherit(ctx, event, session.parentID, remember)

  return {
    context,
    firstInContext: fresh && !session.parentID,
    skills: await skillsToJudge(ctx, context, event),
    model: session.model,
  }
}

/**
 * Start a subagent from its parent: the parent's skills are attached to its
 * first prompt, and the parent's hidden tools stay hidden, so the engine
 * judges only what the subagent adds.
 */
async function inherit(ctx: Plugin.Context, event: SessionPrompt, parentID: SessionID, remember: (set: Hidden) => void): Promise<void> {
  const parent = currentContext(await ctx.session.context({ sessionID: parentID }))
  const present = new Set((event.prompt.skills ?? []).map((skill) => skill.id))
  // Safe: these IDs were attached in the parent, so they are host skill IDs.
  const skills = [...attachedSkills(parent)].filter((id) => !present.has(id as Skill.ID)).map((id) => ({ id: id as Skill.ID }))
  const stored = await ctx.storage.get(storageKey(String(parentID))).catch(() => undefined)
  const hidden = isHiddenList(stored)
    ? (stored.length > 0 ? new Set(stored) : null)
    : await restoreHidden(ctx, parentID).catch(() => null)

  event.prompt.skills = [...(event.prompt.skills ?? []), ...skills]
  remember(hidden)
  await ctx.storage.set(storageKey(String(event.sessionID)), [...(hidden ?? [])])

  if (hidden !== null) event.metadata = { ...event.metadata, [HIDDEN_METADATA_KEY]: [...hidden] }
}

/** Skills not yet in this context, within the catalog cap. */
async function skillsToJudge(ctx: Plugin.Context, context: ReadonlyArray<HistoryMessage>, event: SessionPrompt) {
  const attached = attachedSkills(context, event)

  return (await ctx.skill.list()).data.filter((skill) => !attached.has(skill.id)).slice(0, MAX_CATALOG)
}

/** Skills already in this context: attached to user messages or delivered mid-turn. */
function attachedSkills(context: ReadonlyArray<HistoryMessage>, event?: SessionPrompt): Set<string> {
  return new Set([
    ...context.flatMap((message) => (message.type === "user" ? message.skills ?? [] : [])).map((skill) => skill.id),
    ...context.flatMap((message) => {
      const ids = message.type === "synthetic" ? message.metadata?.[SKILLS_METADATA_KEY] : undefined

      return Array.isArray(ids) ? ids.filter((id): id is string => typeof id === "string") : []
    }),
    ...(event?.prompt.skills ?? []).map((skill) => skill.id),
  ])
}

/**
 * Skills join the prompt as user-selected skills; text joins it as a text
 * attachment. Both reach the first step and add nothing to the system prompt.
 */
function attach(effect: ContextEffect, event: SessionPrompt): void {
  // Safe: the engine only returns skill IDs it received from ctx.skill.list().
  const skills = effect.skills.map((id) => ({ id: id as Skill.ID }))

  event.prompt.skills = [...(event.prompt.skills ?? []), ...skills]

  if (effect.text) {
    const name = `chauffeur-${effect.label.toLowerCase().replace(/[^a-z0-9]+/g, "-")}.txt`

    event.prompt.files = [...(event.prompt.files ?? []), { uri: `data:text/plain;base64,${Buffer.from(effect.text, "utf8").toString("base64")}`, name }]
  }
}

/** Record the context's hidden tools on this message, so a restart restores them. */
function applyTools(effect: Extract<Effect, { type: "tools" }>, event: SessionPrompt, { hidden, remember }: Applying): void {
  const hide = [...new Set([...(hidden ?? []), ...effect.hide])].filter((tool) => !effect.reveal.includes(tool))

  remember(hide.length > 0 ? new Set(hide) : null)
  event.metadata = { ...event.metadata, [HIDDEN_METADATA_KEY]: hide }
}

/** Messages since the most recent compaction, which begins a new context. */
function currentContext(messages: ReadonlyArray<HistoryMessage>): HistoryMessage[] {
  const lastCompaction = messages.findLastIndex((message) => message.type === "compaction")

  return messages.slice(lastCompaction + 1)
}

async function restoreHidden(ctx: Plugin.Context, sessionID: SessionID): Promise<Hidden> {
  const recorded = currentContext(await ctx.session.context({ sessionID }))
    .flatMap((message) => (message.type === "user" || message.type === "synthetic" ? [message.metadata?.[HIDDEN_METADATA_KEY]] : []))
    .findLast((value) => Array.isArray(value))

  return Array.isArray(recorded) && recorded.every((tool) => typeof tool === "string") ? new Set(recorded) : null
}

function isHiddenList(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((tool) => typeof tool === "string")
}

type HostTool = Awaited<ReturnType<Plugin.Context["tool"]["list"]>>[number]

/** A new context's request tools, or a later message's hidden tools. */
async function toolsToJudge(ctx: Plugin.Context, firstInContext: boolean, hidden: Hidden, requestTools: ReadonlySet<string> | null): Promise<HostTool[]> {
  if (firstInContext) return catalog(await ctx.tool.list(), requestTools)
  if (hidden === null) return []

  return catalog((await ctx.tool.list()).filter((tool) => hidden.has(tool.id)), null)
}

/**
 * The tools worth judging, within the catalog cap: `skill` first (it is always
 * hidden), then the other tools the host sends in model requests. Code Mode
 * tools reach the model through `execute`, not the request's tool record, so
 * hiding them would change nothing. Before any request is seen, only tools
 * flagged as Code Mode are left out.
 */
function catalog(tools: ReadonlyArray<HostTool>, inRequests: ReadonlySet<string> | null): HostTool[] {
  return tools
    .filter((tool) => (inRequests ? inRequests.has(tool.id) : tool.options?.codemode !== true))
    .sort((a, b) => Number(b.id === "skill") - Number(a.id === "skill"))
    .slice(0, MAX_CATALOG)
}

function entry(id: string, description: string, content: string): CatalogEntry {
  return {
    id: clip(id, TEXT_CODE_POINTS),
    description: clip(description, DESCRIPTION_CODE_POINTS),
    bytes: Buffer.byteLength(content, "utf8"),
  }
}
