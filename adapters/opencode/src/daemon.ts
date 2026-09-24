import { spawn } from "node:child_process"
import { Context, type Duration, Effect, Schema } from "effect"
import { RpcReply, SignalReply, type HostEffect, type Signal } from "./protocol.js"

const DEFAULT_URL = "http://127.0.0.1:18790"

const RPC_TIMEOUT: Duration.Input = "30 seconds"

// Give the engine's 15s reply deadline time to return its own timeout first.
const SIGNAL_TIMEOUT: Duration.Input = "20 seconds"

const START_ATTEMPTS = 40

export class DaemonError extends Schema.TaggedError<DaemonError>()("DaemonError", { message: Schema.String }) {}

export type DaemonClient = {
  readonly signal: (signal: Signal, timeout?: Duration.Input) => Effect.Effect<ReadonlyArray<HostEffect>, DaemonError>
}

type Endpoint = { readonly url: string; readonly headers: Headers }

type RpcParams = { readonly signal?: Signal }

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

  const healthy = request(endpoint, "health", {}, Schema.Unknown, RPC_TIMEOUT).pipe(
    Effect.as(true),
    Effect.orElseSucceed(() => false),
  )

  if (!(yield* healthy)) {
    if (configured) {
      yield* Effect.logError(`chauffeur: daemon unavailable at ${endpoint.url}`)
    } else if (!(yield* start(healthy))) {
      yield* Effect.logError("chauffeur: daemon did not become healthy within 10 seconds")
    }
  }

  return {
    signal: (value, timeout = SIGNAL_TIMEOUT) =>
      request(endpoint, "signal", { signal: value }, SignalReply, timeout).pipe(Effect.map((reply) => reply.effects)),
  } satisfies DaemonClient
})

export class Daemon extends Context.Service<Daemon, DaemonClient>()("chauffeur/Daemon") {
  static readonly connect: Effect.Effect<DaemonClient> = connect
}

function start(healthy: Effect.Effect<boolean>): Effect.Effect<boolean> {
  return Effect.gen(function* () {
    yield* Effect.sync(() => {
      const child = spawn(process.env.CHAUFFEUR_BIN ?? "chauffeur", ["daemon"], {
        detached: true,
        stdio: "ignore",
        env: process.env,
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

    const envelope = yield* Effect.tryPromise({
      try: () => response.json(),
      catch: () => new DaemonError({ message: "invalid chauffeur RPC response" }),
    }).pipe(
      Effect.flatMap(Schema.decodeUnknownEffect(RpcReply)),
      Effect.mapError(() => new DaemonError({ message: "invalid chauffeur RPC response" })),
    )

    if (envelope.error !== undefined) return yield* new DaemonError({ message: envelope.error })

    if (!response.ok) return yield* new DaemonError({ message: `chauffeur daemon returned ${response.status}` })

    if (envelope.result === undefined) return yield* new DaemonError({ message: "chauffeur daemon response has no result" })

    return yield* Schema.decodeUnknownEffect(result)(envelope.result).pipe(
      Effect.mapError(() => new DaemonError({ message: `chauffeur returned an invalid ${method} result` })),
    )
  }).pipe(
    Effect.timeout(timeout),
    Effect.catchTag("TimeoutError", () => new DaemonError({ message: `chauffeur ${method} timed out` })),
  )
}
