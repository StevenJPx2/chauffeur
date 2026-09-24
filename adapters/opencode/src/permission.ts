import { existsSync, realpathSync } from "node:fs"
import { basename, dirname, resolve } from "node:path"
import type { PermissionEvaluation } from "@opencode/plugin/effect/permission"
import { Effect } from "effect"
import { Daemon } from "./daemon.js"
import { Host } from "./host.js"
import { signal, TEXT_CODE_POINTS, type Resource, type SignalKind } from "./protocol.js"
import { clip, isIntegrationMessage } from "./text.js"

// Permission is synchronous for the host; past this the request asks.
const PERMISSION_TIMEOUT = "600 millis"

const MAX_RESOURCES = 32

const MAX_USER_REQUESTS = 5

// Actions whose resources are file paths; others (shell) carry command text.
const FILE_ACTIONS = new Set(["read", "edit", "write", "patch"])

/**
 * Reports permission requests with resolved paths and recent user requests,
 * and applies the engine's decision. A host denial always stands; any
 * failure asks.
 */
export const installPermission = Effect.gen(function* () {
  const host = yield* Host
  const daemon = yield* Daemon

  yield* host.permission.hook("evaluate", (event) => {
    if (event.effect === "deny") return Effect.void

    const sessionID = String(event.sessionID)

    return Effect.gen(function* () {
      if (event.resources.length > MAX_RESOURCES) return yield* Effect.fail(`more than ${MAX_RESOURCES} resources`)

      const kind = yield* request(event).pipe(Effect.provideService(Host, host))
      const effects = yield* daemon.signal(signal(sessionID, kind), PERMISSION_TIMEOUT)

      for (const effect of effects) {
        if (effect.type !== "permission" || effect.agent_id !== sessionID) continue

        event.effect = effect.decision

        if (effect.message !== null) event.message = effect.message
      }
    }).pipe(Effect.catch((error) => Effect.sync(() => {
      event.effect = "ask"
      event.message = clip(`Chauffeur could not evaluate this request: ${String(error)}`, TEXT_CODE_POINTS)
    })))
  })
})

function request(event: PermissionEvaluation): Effect.Effect<SignalKind, unknown, Host> {
  return Effect.gen(function* () {
    const host = yield* Host
    const workspace = realOrSelf(host.location.project.canonical)
    const history = yield* host.session.context({ sessionID: event.sessionID })

    const userRequests = history
      .flatMap((message) => (message.type === "user" && !isIntegrationMessage(message.metadata) ? [clip(message.text, TEXT_CODE_POINTS)] : []))
      .slice(-MAX_USER_REQUESTS)

    return {
      type: "permission_request",
      action: clip(event.action, TEXT_CODE_POINTS),
      resources: event.resources.map((requested) => FILE_ACTIONS.has(event.action)
        ? resolveResource(workspace, requested)
        : { requested: clip(requested, TEXT_CODE_POINTS), resolved: clip(requested, TEXT_CODE_POINTS) }),
      request: clip(event.message ?? "", TEXT_CODE_POINTS),
      workspace,
      user_requests: userRequests,
    } satisfies SignalKind
  })
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
