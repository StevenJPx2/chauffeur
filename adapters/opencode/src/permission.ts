import { existsSync, realpathSync } from "node:fs"
import { basename, dirname, resolve } from "node:path"
import type { Plugin } from "@opencode/plugin"
import type { PermissionEvaluation } from "@opencode/plugin/promise/permission"
import type { DaemonBridge } from "./daemon.js"
import { signal } from "./model-router.js"
import type { Resource } from "./protocol.js"
import { clip, isIntegrationMessage } from "./text.js"

// Permission is synchronous for the host; past this the request asks.
const PERMISSION_TIMEOUT_MS = 600
const MAX_RESOURCES = 32
const MAX_USER_REQUESTS = 5
const TEXT_CODE_POINTS = 512
// Actions whose resources are file paths; others (shell) carry command text.
const FILE_ACTIONS = new Set(["read", "edit", "write", "patch"])

/**
 * Reports permission requests with resolved paths and recent user requests,
 * and applies the engine's decision. A host denial always stands; any
 * failure asks.
 */
export async function installPermission(ctx: Plugin.Context, daemon: DaemonBridge): Promise<() => Promise<void>> {
  const hook = await ctx.permission.hook("evaluate", async (event) => {
    if (event.effect === "deny") return

    const sessionID = String(event.sessionID)

    try {
      if (event.resources.length > MAX_RESOURCES) throw new Error(`more than ${MAX_RESOURCES} resources`)

      const effects = await daemon.signal(signal(sessionID, await request(ctx, event)), PERMISSION_TIMEOUT_MS)

      for (const effect of effects) {
        if (effect.type !== "permission" || effect.agent_id !== sessionID) continue

        event.effect = effect.decision

        if (effect.message !== null) event.message = effect.message
      }
    } catch (error) {
      event.effect = "ask"
      event.message = clip(`Chauffeur could not evaluate this request: ${String(error)}`, TEXT_CODE_POINTS)
    }
  })

  return () => hook.dispose()
}

async function request(ctx: Plugin.Context, event: PermissionEvaluation) {
  const workspace = realOrSelf(ctx.location.project.canonical)
  const history = await ctx.session.context({ sessionID: event.sessionID })
  const userRequests = history
    .flatMap((message) => (message.type === "user" && !isIntegrationMessage(message.metadata) ? [clip(message.text, TEXT_CODE_POINTS)] : []))
    .slice(-MAX_USER_REQUESTS)

  return {
    type: "permission_request" as const,
    action: clip(event.action, TEXT_CODE_POINTS),
    resources: event.resources.map((requested) => FILE_ACTIONS.has(event.action)
      ? resolveResource(workspace, requested)
      : { requested: clip(requested, TEXT_CODE_POINTS), resolved: clip(requested, TEXT_CODE_POINTS) }),
    request: clip(event.message ?? "", TEXT_CODE_POINTS),
    workspace,
    user_requests: userRequests,
  }
}

/** Absolute, with symlinks resolved for the path or its parent when they exist. */
function resolveResource(workspace: string, requested: string): Resource {
  const candidate = resolve(workspace, requested)
  let resolved = candidate

  try {
    resolved = existsSync(candidate) ? realpathSync(candidate) : resolve(realpathSync(dirname(candidate)), basename(candidate))
  } catch {
    // Unresolvable paths keep their lexical form; core checks containment lexically.
  }

  return { requested: clip(requested, TEXT_CODE_POINTS), resolved: clip(resolved, TEXT_CODE_POINTS) }
}

function realOrSelf(path: string): string {
  try {
    return realpathSync(path)
  } catch {
    return path
  }
}
