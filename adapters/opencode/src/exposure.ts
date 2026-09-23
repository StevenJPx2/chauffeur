import type { Plugin, Skill } from "@opencode/plugin"
import type { SessionPrompt } from "@opencode/plugin/promise/session"
import type { DaemonBridge } from "./daemon.js"
import { signal } from "./model-router.js"
import type { CatalogEntry, Effect } from "./protocol.js"
import { type CodeModeCatalog, codeModeCatalog, surface, surfacedIn } from "./code-mode.js"
import { SKILL_METADATA_KEY } from "./skills.js"
import { clip, isIntegrationMessage } from "./text.js"

const MAX_SESSIONS = 256
const MAX_CATALOG = 128
const TEXT_CODE_POINTS = 512
// 100 code points stays within the engine's 400-byte description bound.
const DESCRIPTION_CODE_POINTS = 100
// Admission waits for exposure; past this the prompt proceeds unenhanced.
const EXPOSURE_TIMEOUT_MS = 6_000
const HIDDEN_METADATA_KEY = "chauffeur.hidden"

type HistoryMessage = Awaited<ReturnType<Plugin.Context["session"]["context"]>>[number]
type SessionID = Parameters<Plugin.Context["session"]["context"]>[0]["sessionID"]
/** Tools hidden for the context; `null` records that nothing is hidden. */
type Hidden = ReadonlySet<string> | null

/**
 * Senses user prompts with the skill and tool catalogs, attaches the skills
 * the engine selects to the prompt, and hides the tools the engine judged
 * unneeded for the context, bringing them back when the engine says so.
 * Everything is derived from session history, so a restart loses nothing.
 */
export async function installExposure(ctx: Plugin.Context, daemon: DaemonBridge): Promise<() => Promise<void>> {
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
      set = await restoreHidden(ctx, id).catch(() => null)
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
      if (firstInContext) remember(sessionID, null)

      const hidden = firstInContext ? null : await current(sessionID, event.sessionID)
      const tools = await toolsToJudge(ctx, firstInContext, hidden, requestTools)
      const codeMode = codeModeCatalog(await ctx.tool.list(), requestTools, surfacedIn(context))
      const effects = await daemon.signal(signal(sessionID, {
        type: "user_message",
        text: clip(event.prompt.text, TEXT_CODE_POINTS),
        first_in_context: firstInContext,
        skills: skills.map((skill) => entry(skill.id, skill.description ?? skill.name, skill.content)),
        tools: tools.map((tool) => entry(tool.id, tool.description, "")),
        model: model ? { provider: model.providerID, model: model.id } : null,
        code_mode: codeMode.entries,
      }), EXPOSURE_TIMEOUT_MS)

      await apply(ctx, event, effects, { hidden: hiddenTools.get(sessionID) ?? null, remember: (set) => remember(sessionID, set), codeMode })
    } catch (error) {
      // Exposure fails open: the prompt is admitted unenhanced.
      console.error(`[chauffeur] exposure unavailable: ${String(error)}`)
    }
  })

  return async () => {
    await prompt.dispose()
    await context.dispose()
    hiddenTools.clear()
  }
}

/** What applying the engine's effects to one prompt needs. */
type Applying = { hidden: Hidden; remember: (set: Hidden) => void; codeMode: CodeModeCatalog }

async function apply(ctx: Plugin.Context, event: SessionPrompt, effects: Effect[], applying: Applying): Promise<void> {
  for (const effect of effects) {
    if (effect.agent_id !== String(event.sessionID)) continue

    if (effect.type === "switch_model") {
      // Switching back to the model the agent left on a usage limit.
      await ctx.session.switchModel({ sessionID: event.sessionID, model: { providerID: effect.model.provider, id: effect.model.model } })
    } else if (effect.type === "surface_tools") {
      surface(event, effect.namespaces, applying.codeMode)
    } else {
      applyToPrompt(effect, event, applying.hidden, applying.remember)
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
  const hidden = await restoreHidden(ctx, parentID).catch(() => null)

  event.prompt.skills = [...(event.prompt.skills ?? []), ...skills]
  remember(hidden)

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
      const id = message.type === "synthetic" ? message.metadata?.[SKILL_METADATA_KEY] : undefined

      return typeof id === "string" ? [id] : []
    }),
    ...(event?.prompt.skills ?? []).map((skill) => skill.id),
  ])
}

function applyToPrompt(effect: Effect, event: SessionPrompt, hidden: Hidden, remember: (set: Hidden) => void): void {
  if (effect.type === "attach_skills") {
    // Safe: the engine only returns IDs it received from ctx.skill.list(), the host's skill registry.
    const attach = effect.skills.map((id) => ({ id: id as Skill.ID }))

    event.prompt.skills = [...(event.prompt.skills ?? []), ...attach]
  } else if (effect.type === "hide_tools") {
    remember(new Set(effect.tools))
    event.metadata = { ...event.metadata, [HIDDEN_METADATA_KEY]: effect.tools }
  } else if (effect.type === "reveal_tools" && hidden !== null) {
    // Recorded on this message, so a restart restores the smaller set.
    const remaining = [...hidden].filter((tool) => !effect.tools.includes(tool))

    remember(new Set(remaining))
    event.metadata = { ...event.metadata, [HIDDEN_METADATA_KEY]: remaining }
  }
}

/** Messages since the most recent compaction, which begins a new context. */
function currentContext(messages: ReadonlyArray<HistoryMessage>): HistoryMessage[] {
  const lastCompaction = messages.findLastIndex((message) => message.type === "compaction")

  return messages.slice(lastCompaction + 1)
}

async function restoreHidden(ctx: Plugin.Context, sessionID: SessionID): Promise<Hidden> {
  const recorded = currentContext(await ctx.session.context({ sessionID }))
    .flatMap((message) => (message.type === "user" ? [message.metadata?.[HIDDEN_METADATA_KEY]] : []))
    .findLast((value) => Array.isArray(value))

  return Array.isArray(recorded) && recorded.every((tool) => typeof tool === "string") ? new Set(recorded) : null
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
