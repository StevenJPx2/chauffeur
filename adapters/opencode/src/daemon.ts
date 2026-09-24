import { spawn } from "node:child_process"
import type { Effect, RpcResponse, Signal } from "./protocol.js"

const DEFAULT_URL = "http://127.0.0.1:18790"
const RPC_TIMEOUT_MS = 30_000
// Matches the engine's reply timeout; model routing blocks the host's retry decision.
const SIGNAL_TIMEOUT_MS = 15_000
const PERMISSION_DECISIONS = ["allow", "deny", "ask"] as const
const DELIVERIES = ["prompt", "steer", "resume", "wait"] as const

export class DaemonBridge {
  readonly url = (process.env.CHAUFFEUR_DAEMON_URL ?? DEFAULT_URL).replace(/\/$/, "")
  private readonly token = process.env.CHAUFFEUR_DAEMON_TOKEN

  async start(): Promise<void> {
    if (await this.healthy()) return
    if (process.env.CHAUFFEUR_DAEMON_URL) throw new Error(`chauffeur daemon unavailable at ${this.url}`)

    const child = spawn(process.env.CHAUFFEUR_BIN ?? "chauffeur", ["daemon"], {
      detached: true,
      stdio: "ignore",
      env: process.env,
    })
    child.unref()

    for (let attempt = 0; attempt < 40; attempt += 1) {
      await delay(250)

      if (await this.healthy()) return
    }

    throw new Error("chauffeur daemon did not become healthy within 10 seconds")
  }

  async signal(signal: Signal, timeoutMs = SIGNAL_TIMEOUT_MS): Promise<Effect[]> {
    const result = await this.rpc<unknown>("signal", { signal }, timeoutMs)

    if (!isRecord(result) || !Array.isArray(result.effects) || !result.effects.every(isEffect)) {
      throw new Error("chauffeur returned invalid effects")
    }

    return result.effects
  }

  private async healthy(): Promise<boolean> {
    try {
      await this.rpc("health", {})
      return true
    } catch {
      return false
    }
  }

  private async rpc<T>(method: string, params: unknown, timeoutMs = RPC_TIMEOUT_MS): Promise<T> {
    const response = await fetch(`${this.url}/rpc`, {
      method: "POST",
      headers: { "content-type": "application/json", ...this.headers() },
      body: JSON.stringify({ id: 1, method, params }),
      signal: AbortSignal.timeout(timeoutMs),
    })
    const envelope = decodeRpc<T>(await response.json())

    if (envelope.error) throw new Error(envelope.error)
    if (!response.ok) throw new Error(`chauffeur daemon returned ${response.status}`)
    if (envelope.result === undefined) throw new Error("chauffeur daemon response has no result")

    return envelope.result
  }

  private headers(): Record<string, string> {
    return this.token ? { authorization: `Bearer ${this.token}` } : {}
  }
}

function decodeRpc<T>(value: unknown): RpcResponse<T> {
  if (!isRecord(value)) throw new Error("invalid chauffeur RPC response")

  return {
    ...(typeof value.id === "number" ? { id: value.id } : {}),
    ...(typeof value.error === "string" ? { error: value.error } : {}),
    ...(value.result !== undefined ? { result: value.result as T } : {}),
  }
}

function isEffect(value: unknown): value is Effect {
  if (!isRecord(value) || typeof value.agent_id !== "string") return false
  if (value.type === "gate") return typeof value.deliver === "boolean"
  if (value.type === "tools") return isStringArray(value.hide) && isStringArray(value.reveal)
  if (value.type === "context") {
    return isOneOf(value.delivery, DELIVERIES)
      && typeof value.label === "string"
      && isStringArray(value.skills)
      && (value.text === null || typeof value.text === "string")
  }
  if (value.type === "permission") {
    return isOneOf(value.decision, PERMISSION_DECISIONS) && (value.message === null || typeof value.message === "string")
  }

  return value.type === "model"
    && (value.model === null || (isRecord(value.model) && typeof value.model.provider === "string" && typeof value.model.model === "string"))
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === "string")
}

function isOneOf<const Values extends readonly string[]>(
  value: unknown,
  values: Values,
): value is Values[number] {
  return typeof value === "string" && values.some((candidate) => candidate === value)
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
