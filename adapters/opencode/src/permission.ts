import { existsSync, realpathSync } from "node:fs"
import { basename, dirname, resolve } from "node:path"
import type { PermissionEvaluation } from "@opencode/plugin/effect/permission"
import { Effect, Schedule } from "effect"
import { Daemon, type DaemonError } from "./daemon.js"
import { type HistoryMessage, Host } from "./host.js"
import { RESOURCE_BYTES, RESOURCES_TOTAL_BYTES, signal, TEXT_CODE_POINTS, type Resource, type SignalKind } from "./protocol.js"
import { clip, clipStart, isIntegrationMessage, userText } from "./text.js"
import { fill, Texts } from "./texts.js"

// Permission is synchronous for the host; past this the request asks. The
// budget includes session-history lookup and queued daemon work as well as
// judgment. Two seconds can expire before an otherwise valid reply arrives.
const PERMISSION_TIMEOUT = "5 seconds"

// A daemon mid-restart refuses connections for about a second.
const UNREACHABLE_RETRIES = 2

const UNREACHABLE_RETRY_DELAY = "500 millis"

const MAX_RESOURCES = 32

const MAX_USER_REQUESTS = 5

// Actions whose resources are file paths; others (shell) carry command text.
const FILE_ACTIONS = new Set(["read", "edit", "write", "patch"])

/**
 * Reports permission requests with the host's own decision, resolved paths,
 * and recent user requests, and applies the engine's decision. The engine
 * judges what the host would ask about and vetoes irreversible harm in what
 * it would allow. A host denial always stands; a failure keeps the host's
 * allow and asks otherwise.
 */
export const installPermission = Effect.gen(function* () {
  const host = yield* Host
  const daemon = yield* Daemon
  const texts = yield* Texts

  yield* host.permission.hook("evaluate", (event) => {
    if (event.effect === "deny") return Effect.void

    const sessionID = String(event.sessionID)
    const hostDecision = event.effect

    return Effect.gen(function* () {
      if (event.resources.length > MAX_RESOURCES) return yield* Effect.fail(`more than ${MAX_RESOURCES} resources`)

      const kind = yield* request(event, hostDecision).pipe(Effect.provideService(Host, host))

      const effects = yield* daemon.signal(signal(sessionID, kind), PERMISSION_TIMEOUT).pipe(
        Effect.retry({ times: UNREACHABLE_RETRIES, schedule: Schedule.spaced(UNREACHABLE_RETRY_DELAY), while: unreachable }),
      )

      for (const effect of effects) {
        if (effect.type !== "permission" || effect.agent_id !== sessionID) continue

        event.effect = effect.decision

        if (effect.message !== null) event.message = effect.message
      }
    }).pipe(Effect.timeout(PERMISSION_TIMEOUT), Effect.catch((error) => {
      // Chauffeur only vetoes what the host allows, so a failure leaves the allow.
      if (hostDecision === "allow") {
        event.effect = "allow"

        return Effect.logWarning(`chauffeur: permission check failed; keeping the host's allow: ${String(error)}`)
      }

      return Effect.sync(() => {
        event.effect = "ask"
        event.message = clip(fill(texts.current().permission.failed, { error: String(error) }), TEXT_CODE_POINTS)
      })
    }))
  })
})

function unreachable(error: DaemonError): boolean {
  return error.message.startsWith("chauffeur daemon unreachable")
}

function request(event: PermissionEvaluation, hostDecision: "allow" | "ask"): Effect.Effect<SignalKind, unknown, Host> {
  return Effect.gen(function* () {
    const host = yield* Host
    const workspace = realOrSelf(host.location.project.canonical)
    const history = yield* host.session.context({ sessionID: event.sessionID })

    const userRequests = history
      .flatMap((message) => (message.type === "user" && !isIntegrationMessage(message.metadata) ? [userText(message.text)] : []))
      .slice(-MAX_USER_REQUESTS)

    const resources = yield* Effect.try({
      try: () => withinTotal(event.resources.map((requested) => FILE_ACTIONS.has(event.action)
        ? resolveResource(workspace, requested)
        : resource(requested, requested))),
      catch: (error) => String(error),
    })

    return {
      type: "permission_request",
      action: clip(event.action, TEXT_CODE_POINTS),
      resources,
      request: reason(event.message, history),
      workspace,
      user_requests: userRequests,
      host_decision: hostDecision,
    } satisfies SignalKind
  })
}

/**
 * Why the agent wants this: the host's own message when it has one, else what
 * the agent said just before the call, which is its stated reason.
 */
function reason(message: string | undefined, history: ReadonlyArray<HistoryMessage>): string {
  if (message !== undefined && message.trim() !== "") return clip(message, TEXT_CODE_POINTS)

  const reply = history.findLast((entry) => entry.type === "assistant")

  if (reply?.type !== "assistant") return ""

  const said = reply.content.flatMap((part) => (part.type === "text" ? [part.text] : [])).join("\n").trim()

  return clipStart(said, TEXT_CODE_POINTS)
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

  return resource(requested, resolved)
}

/** Never clip a safety target: hidden command suffixes and path tails matter. */
function resource(requested: string, resolved: string): Resource {
  if (Buffer.byteLength(requested, "utf8") > RESOURCE_BYTES || Buffer.byteLength(resolved, "utf8") > RESOURCE_BYTES) {
    throw new Error(`permission resource exceeds ${RESOURCE_BYTES} bytes`)
  }

  return { requested, resolved }
}

function withinTotal(resources: Resource[]): Resource[] {
  const total = resources.reduce((sum, { requested, resolved }) => sum + Buffer.byteLength(requested, "utf8") + Buffer.byteLength(resolved, "utf8"), 0)

  if (total > RESOURCES_TOTAL_BYTES) throw new Error(`permission resources exceed ${RESOURCES_TOTAL_BYTES} bytes together`)

  return resources
}

function realOrSelf(path: string): string {
  try {
    return realpathSync(path)
  } catch {
    return path
  }
}
