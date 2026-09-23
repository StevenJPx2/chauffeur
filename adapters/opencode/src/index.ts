import { existsSync, realpathSync } from "node:fs"
import { basename, dirname, isAbsolute, relative, resolve, sep } from "node:path"
import { Plugin } from "@opencode/plugin"
import type { PermissionEvaluation } from "@opencode/plugin/promise/permission"
import { DaemonBridge } from "./daemon.js"
import type { AgentContext, SkillContext, SkillResult, Target, ToolCall } from "./protocol.js"

const HISTORY_LIMIT = 40
const RESOURCE_LIMIT = 32
const RESOURCE_BYTES_LIMIT = 2_048
const STATE_BYTES_LIMIT = 16_384
type PluginContext = Plugin.Context

export default Plugin.define({
  id: "chauffeur",
  async setup(ctx) {
    const daemon = new DaemonBridge()
    const controller = new AbortController()
    const histories = new Map<string, ToolCall[]>()
    const listeners = new Map<string, AbortController>()
    const idleRegistrations: Array<{ dispose: () => Promise<void> }> = []

    let skillIDs: string[] = []
    let startupFailure: string | null = null

    try {
      await daemon.start()
      skillIDs = await daemon.skills()
    } catch (error) {
      startupFailure = errorText(error)
      console.error(`[chauffeur] skill daemon unavailable: ${startupFailure}`)
    }

    if (process.env.CHAUFFEUR_IDLE_STEERING === "true") {
      const toolHook = await ctx.tool.hook("execute.after", (event) => {
        if (event.status !== "completed") return

        const history = histories.get(event.sessionID) ?? []
        history.push({
          name: event.tool,
          summary: summarize(event.input),
          at: Math.floor(Date.now() / 1000),
        })
        histories.set(event.sessionID, history.slice(-HISTORY_LIMIT))
      })

      idleRegistrations.push(toolHook)
      void observeIdle(ctx, daemon, histories, listeners, controller.signal)
    }
    const permissionHook = await ctx.permission.hook("evaluate", (event) =>
      enforcePermission(ctx, daemon, skillIDs, startupFailure, event),
    )

    return async () => {
      controller.abort()

      for (const listener of listeners.values()) listener.abort()

      for (const registration of idleRegistrations) await registration.dispose()

      await permissionHook.dispose()
    }
  },
})

async function observeIdle(
  ctx: PluginContext,
  daemon: DaemonBridge,
  histories: Map<string, ToolCall[]>,
  listeners: Map<string, AbortController>,
  signal: AbortSignal,
): Promise<void> {
  for await (const event of ctx.event.subscribe({ signal })) {
    if (event.type !== "session.status" || event.data.status.type !== "idle") continue

    const sessionID = event.data.sessionID
    const target = targetFor(sessionID)

    ensureListener(ctx, daemon, target, listeners)

    try {
      await daemon.steer(target, contextFor(sessionID, histories.get(sessionID) ?? []))
    } catch (error) {
      console.error(`[chauffeur] idle steering failed: ${error instanceof Error ? error.message : String(error)}`)
    }
  }
}

async function enforcePermission(
  ctx: PluginContext,
  daemon: DaemonBridge,
  skillIDs: string[],
  startupFailure: string | null,
  event: PermissionEvaluation,
): Promise<void> {
  if (event.effect === "deny") return

  if (startupFailure !== null) {
    event.effect = "ask"
    event.message = `Chauffeur cannot verify its skill contracts: ${startupFailure}`

    return
  }
  if (skillIDs.length === 0) return

  try {
    const context = await permissionContext(ctx, event)
    const results: SkillResult[] = []

    for (const skillID of skillIDs) {
      results.push(await daemon.evaluateSkill(skillID, context))
    }

    applySkillResults(event, results)
  } catch (error) {
    event.effect = "ask"
    event.message = `Chauffeur could not evaluate its skill contracts: ${errorText(error)}`
  }
}

async function permissionContext(
  ctx: PluginContext,
  event: PermissionEvaluation,
): Promise<SkillContext> {
  if (Buffer.byteLength(event.action, "utf8") > 120 || Buffer.byteLength(event.sessionID, "utf8") > 128) {
    throw new Error("permission event identifiers exceed Chauffeur limits")
  }

  const messages = await ctx.session.context({ sessionID: event.sessionID })
  const requests = messages
    .slice(-20)
    .flatMap((message) => message.type === "user" ? [boundedUtf8(message.text, 1_024)] : [])
    .slice(-5)
  const userText = requests.join("\n").toLowerCase()
  const resourcesComplete = event.resources.length > 0
    && event.resources.length <= RESOURCE_LIMIT
    && event.resources.every((resource) => resource.length > 0
      && Buffer.byteLength(resource, "utf8") <= RESOURCE_BYTES_LIMIT)
  const resources = event.resources
    .slice(0, RESOURCE_LIMIT)
    .map((resource) => boundedUtf8(resource, RESOURCE_BYTES_LIMIT))
  const stateResources = resources.map((resource) => boundedUtf8(resource, 256))
  const request = boundedUtf8(event.message ?? "", 1_024)
  const state = boundedUtf8([
    "Recent user requests:",
    ...requests,
    `Permission action: ${event.action}`,
    `Resources: ${stateResources.join(", ")}`,
    `Permission request: ${request}`,
  ].join("\n"), STATE_BYTES_LIMIT)
  const resourcePresent = resourcesComplete && resources.length > 0
  const withinWorkspace = resourcePresent
    && resources.every((resource) => isWithinWorkspace(ctx.location.project.canonical, resource))
  const namedByUser = resourcePresent
    && resources.every((resource) => userNamedResource(userText, resource))

  return {
    event: "permission.evaluate",
    action: event.action,
    agent_id: event.sessionID,
    occurred_at: Math.floor(Date.now() / 1_000),
    state,
    evidence: {
      resource_present: resourcePresent,
      resource_within_workspace: withinWorkspace,
      resource_named_by_user: namedByUser,
    },
  }
}

function userNamedResource(request: string, resource: string): boolean {
  const normalized = resource.toLowerCase()
  const fileName = basename(resource).toLowerCase()

  return request.includes(normalized) || request.includes(fileName)
}

function isWithinWorkspace(workspace: string, resource: string): boolean {
  try {
    const realWorkspace = realpathSync(workspace)
    const candidate = resolve(workspace, resource)
    const realResource = existsSync(candidate)
      ? realpathSync(candidate)
      : resolve(realpathSync(dirname(candidate)), basename(candidate))
    const fromWorkspace = relative(realWorkspace, realResource)

    return fromWorkspace === ""
      || (fromWorkspace !== ".." && !fromWorkspace.startsWith(`..${sep}`) && !isAbsolute(fromWorkspace))
  } catch {
    return false
  }
}

function applySkillResults(event: PermissionEvaluation, results: SkillResult[]): void {
  let selected: { result: SkillResult; effect: "allow" | "deny" | "ask" } | undefined

  for (const result of results) {
    if (result.status === "unmatched" || result.effect === null) continue

    const effect = permissionEffect(result.effect)

    if (!selected || permissionRank(effect) < permissionRank(selected.effect)) {
      selected = { result, effect }
    }
  }

  if (!selected) return

  event.effect = selected.effect

  if (selected.effect !== "allow") {
    event.message = selected.result.reminder
      ?? selected.result.message
      ?? "Chauffeur skill policy requires review."
  }
}

function permissionEffect(effect: SkillResult["effect"]): "allow" | "deny" | "ask" {
  if (effect === "allow" || effect === "deny") return effect

  return "ask"
}

function permissionRank(effect: PermissionEvaluation["effect"]): number {
  if (effect === "deny") return 0
  if (effect === "ask") return 1

  return 2
}

function boundedUtf8(value: string, maxBytes: number): string {
  let output = ""
  let bytes = 0

  for (const character of value) {
    const characterBytes = Buffer.byteLength(character, "utf8")

    if (bytes + characterBytes > maxBytes) break

    output += character
    bytes += characterBytes
  }

  return output
}

function errorText(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error)

  return boundedUtf8(message, 500)
}

function ensureListener(
  ctx: PluginContext,
  daemon: DaemonBridge,
  target: Target,
  listeners: Map<string, AbortController>,
): void {
  if (listeners.has(target.id)) return

  const controller = new AbortController()
  listeners.set(target.id, controller)

  void daemon.subscribe(target, controller.signal, async (queued) => {
    for (const item of queued) {
      await ctx.session.prompt({ sessionID: target.id, text: item.reminder.text })
      await daemon.acknowledge(target, [item.id])
    }
  })
}

function contextFor(sessionID: string, toolHistory: ToolCall[]): AgentContext {
  const status = toolHistory.some((call) => call.name === "github_open_pr") ? "in_review" : "implementing"
  const source = toolHistory.some((call) => call.name.startsWith("jira_"))
    ? "jira"
    : toolHistory.some((call) => call.name.startsWith("github_"))
      ? "github"
      : ""

  return {
    agent_id: sessionID,
    status,
    source,
    tool_history: toolHistory,
    notifications: [],
    hooks: [],
    idle_at: Math.floor(Date.now() / 1000),
  }
}

function targetFor(sessionID: string): Target {
  return { kind: "opencode-session", id: sessionID }
}

function summarize(input: unknown): string {
  try {
    return JSON.stringify(input).slice(0, 120)
  } catch {
    return "unserializable input"
  }
}
