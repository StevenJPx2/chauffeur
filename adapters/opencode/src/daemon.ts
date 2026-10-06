import { spawn } from "node:child_process"
import { Context, type Duration, Effect, Option, Schema } from "effect"
import { daemonLaunch, PLUGIN_VERSION } from "./package.js"
import { RpcReply, RulebooksReply, SignalReply, type HostEffect, type RulebookEntry, type Signal } from "./protocol.js"

const DEFAULT_URL = "http://127.0.0.1:18790"

const RPC_TIMEOUT: Duration.Input = "30 seconds"

// Give the engine's 15s reply deadline time to return its own timeout first.
const SIGNAL_TIMEOUT: Duration.Input = "20 seconds"

const START_ATTEMPTS = 40

/** Checks, 250 ms apart, for a replaced daemon to stop answering. */
const STOP_ATTEMPTS = 20

export class DaemonError extends Schema.TaggedError<DaemonError>()("DaemonError", { message: Schema.String }) {}

export type DaemonClient = {
  readonly signal: (signal: Signal, timeout?: Duration.Input) => Effect.Effect<ReadonlyArray<HostEffect>, DaemonError>
  /** The rulebooks the user can start in `workspace`, offered as slash commands. */
  readonly rulebooks: (workspace: string) => Effect.Effect<ReadonlyArray<RulebookEntry>, DaemonError>
  /** The wording `host` shows the agent, as the daemon serves it; the caller decodes it. */
  readonly texts: (host: string) => Effect.Effect<unknown, DaemonError>
}

type Endpoint = { readonly url: string; readonly headers: Headers }

type RpcParams = { readonly signal?: Signal; readonly workspace?: string; readonly host?: string }

/**
 * Connect to the configured daemon, or start a local one when none answers.
 * Never fails: while the daemon is down, each capability applies its own
 * failure posture to the errors its signals return.
 */
const connect = Effect.gen(function* () {
  const configured = process.env.CHAUFFEUR_DAEMON_URL
  const headers = new Headers({ "content-type": "application/json" })
  const token = process.env.CHAUFFEUR_DAEMON_TOKEN

  if (token) headers.set("authorization", `Bearer ${token}`)

  const endpoint: Endpoint = { url: (configured ?? DEFAULT_URL).replace(/\/$/, ""), headers }

  const found = request(endpoint, "health", {}, Health, RPC_TIMEOUT).pipe(Effect.option, Effect.map(Option.getOrUndefined))
  const healthy = found.pipe(Effect.map((daemon) => daemon !== undefined))
  const daemon = yield* found

  yield* settle(endpoint, startupStep(daemon, configured !== undefined), daemon?.version, healthy)

  return {
    signal: (value, timeout = SIGNAL_TIMEOUT) =>
      request(endpoint, "signal", { signal: value }, SignalReply, timeout).pipe(Effect.map((reply) => reply.effects)),
    rulebooks: (workspace) =>
      request(endpoint, "rulebooks", { workspace }, RulebooksReply, RPC_TIMEOUT).pipe(Effect.map((reply) => reply.rulebooks)),
    texts: (host) => request(endpoint, "texts", { host }, Schema.Unknown, RPC_TIMEOUT),
  } satisfies DaemonClient
})

export class Daemon extends Context.Service<Daemon, DaemonClient>()("chauffeur/Daemon") {
  static readonly connect: Effect.Effect<DaemonClient> = connect
}

/** A daemon's `health` reply; daemons before versioned releases send no version. */
const Health = Schema.Struct({ ok: Schema.Boolean, version: Schema.optional(Schema.String) })

/**
 * What the plugin does with the daemon it finds at startup. It starts one when
 * none answers and replaces one of another version, but never touches a daemon
 * the user runs at `CHAUFFEUR_DAEMON_URL`.
 */
export type StartupStep = "use" | "start" | "replace" | "unavailable" | "mismatch"

export function startupStep(found: { readonly version?: string | undefined } | undefined, configured: boolean, version: string = PLUGIN_VERSION): StartupStep {
  if (found === undefined) return configured ? "unavailable" : "start"

  if (found.version === version) return "use"

  return configured ? "mismatch" : "replace"
}

function settle(endpoint: Endpoint, step: StartupStep, found: string | undefined, healthy: Effect.Effect<boolean>): Effect.Effect<void> {
  const versions = `the daemon at ${endpoint.url} is ${found ?? "unversioned"}; this plugin is ${PLUGIN_VERSION}`

  if (step === "unavailable") return Effect.logError(`chauffeur: daemon unavailable at ${endpoint.url}`)

  if (step === "mismatch") return Effect.logWarning(`chauffeur: ${versions}. Update the daemon you run, or unset CHAUFFEUR_DAEMON_URL.`)

  if (step === "replace") return replace(endpoint, healthy, versions)

  if (step === "start") return started(healthy)

  return Effect.void
}

/** Stop a daemon of another version, wait for it to go, and start this plugin's own. */
function replace(endpoint: Endpoint, healthy: Effect.Effect<boolean>, versions: string): Effect.Effect<void> {
  return Effect.gen(function* () {
    const stopping = yield* request(endpoint, "shutdown", {}, Schema.Unknown, RPC_TIMEOUT).pipe(
      Effect.as(true),
      Effect.orElseSucceed(() => false),
    )

    // A daemon too old to stop on request keeps serving.
    if (!stopping) return yield* Effect.logWarning(`chauffeur: ${versions}, and it cannot be replaced; using it.`)

    yield* Effect.logInfo(`chauffeur: ${versions}; replacing it.`)

    for (let attempt = 0; attempt < STOP_ATTEMPTS && (yield* healthy); attempt += 1) yield* Effect.sleep("250 millis")

    yield* started(healthy)
  })
}

function started(healthy: Effect.Effect<boolean>): Effect.Effect<void> {
  return start(healthy).pipe(
    Effect.flatMap((up) => up ? Effect.void : Effect.logError("chauffeur: daemon did not become healthy within 10 seconds")),
  )
}

function start(healthy: Effect.Effect<boolean>): Effect.Effect<boolean> {
  return Effect.gen(function* () {
    yield* Effect.sync(() => {
      const launch = daemonLaunch(process.env)

      const child = spawn(launch.bin, ["daemon"], {
        detached: true,
        stdio: "ignore",
        env: launch.env,
      })

      // A missing binary is reported by the health checks below.
      child.once("error", () => undefined)
      child.unref()
    })

    for (let attempt = 0; attempt < START_ATTEMPTS; attempt += 1) {
      yield* Effect.sleep("250 millis")

      if (yield* healthy) return true
    }

    return false
  })
}

function request<S extends Schema.Top>(
  endpoint: Endpoint,
  method: string,
  params: RpcParams,
  result: S,
  timeout: Duration.Input,
): Effect.Effect<S["Type"], DaemonError, S["DecodingServices"]> {
  return Effect.gen(function* () {
    const response = yield* Effect.tryPromise({
      try: (abort) => fetch(`${endpoint.url}/rpc`, {
        method: "POST",
        headers: endpoint.headers,
        body: JSON.stringify({ id: 1, method, params }),
        signal: abort,
      }),
      catch: (cause) => new DaemonError({ message: `chauffeur daemon unreachable: ${String(cause)}` }),
    })

    const status = new DaemonError({ message: `chauffeur daemon returned ${response.status}` })

    const envelope = yield* Effect.tryPromise({
      try: () => response.json(),
      catch: () => new DaemonError({ message: "invalid chauffeur RPC response" }),
    }).pipe(
      Effect.flatMap(Schema.decodeUnknownEffect(RpcReply)),
      Effect.mapError(() => response.ok ? new DaemonError({ message: "invalid chauffeur RPC response" }) : status),
    )

    if (envelope.error !== undefined) return yield* new DaemonError({ message: envelope.error })

    if (!response.ok) return yield* status

    if (envelope.result === undefined) return yield* new DaemonError({ message: "chauffeur daemon response has no result" })

    return yield* Schema.decodeUnknownEffect(result)(envelope.result).pipe(
      Effect.mapError(() => new DaemonError({ message: `chauffeur returned an invalid ${method} result` })),
    )
  }).pipe(
    Effect.timeout(timeout),
    Effect.catchTag("TimeoutError", () => new DaemonError({ message: `chauffeur ${method} timed out` })),
  )
}
