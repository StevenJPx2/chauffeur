import { spawn } from "node:child_process"
import type {
  AgentContext,
  EventFrame,
  QueuedReminder,
  RpcResponse,
  SkillContext,
  SkillResult,
  Target,
} from "./protocol.js"

const DEFAULT_URL = "http://127.0.0.1:18788"
const MAX_FRAME_BYTES = 1_000_000
const SKILL_EFFECTS = ["allow", "deny", "ask", "prompt", "remind"] as const
const SKILL_STATUSES = ["unmatched", "missing_evidence", "cooldown", "evaluated", "judge_failure"] as const
const SKILL_BRANCHES = ["positive", "negative", "uncertain"] as const

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

  async steer(target: Target, context: AgentContext): Promise<void> {
    await this.rpc("steer", { target, context })
  }

  async acknowledge(target: Target, reminderIDs: string[]): Promise<void> {
    await this.rpc("reminders.acknowledge", { target, reminder_ids: reminderIDs })
  }

  async skills(): Promise<string[]> {
    const result = await this.rpc<unknown>("skills.list", {})

    if (!Array.isArray(result) || !result.every((id) => typeof id === "string")) {
      throw new Error("chauffeur returned invalid skill IDs")
    }

    return result
  }

  async evaluateSkill(skillID: string, context: SkillContext): Promise<SkillResult> {
    const result = await this.rpc<unknown>("skill.evaluate", { skill_id: skillID, context })

    if (!isSkillResult(result)) throw new Error("chauffeur returned an invalid skill result")

    return result
  }

  async subscribe(
    target: Target,
    signal: AbortSignal,
    deliver: (reminders: QueuedReminder[]) => Promise<void>,
  ): Promise<void> {
    while (!signal.aborted) {
      try {
        await this.subscribeOnce(target, signal, deliver)
      } catch (error) {
        if (signal.aborted) return

        console.error(`[chauffeur] event stream disconnected: ${error instanceof Error ? error.message : String(error)}`)
      }

      await delay(1_000)
    }
  }

  private async subscribeOnce(
    target: Target,
    signal: AbortSignal,
    deliver: (reminders: QueuedReminder[]) => Promise<void>,
  ): Promise<void> {
    const query = new URLSearchParams({ target: JSON.stringify(target) })
    const response = await fetch(`${this.url}/events?${query}`, {
      headers: this.headers(),
      signal,
    })

    if (!response.ok || !response.body) throw new Error(`chauffeur event stream returned ${response.status}`)

    for await (const data of sseData(response.body, signal)) {
      const frame = decodeFrame(data)

      if (frame?.type === "event") await deliver(frame.reminders)
    }
  }

  private async healthy(): Promise<boolean> {
    try {
      await this.rpc("health", {})
      return true
    } catch {
      return false
    }
  }

  private async rpc<T>(method: string, params: unknown): Promise<T> {
    const response = await fetch(`${this.url}/rpc`, {
      method: "POST",
      headers: { "content-type": "application/json", ...this.headers() },
      body: JSON.stringify({ id: 1, method, params }),
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

async function* sseData(stream: ReadableStream<Uint8Array>, signal: AbortSignal): AsyncGenerator<string> {
  const reader = stream.getReader()
  const decoder = new TextDecoder()
  let buffer = ""

  try {
    while (!signal.aborted) {
      const { done, value } = await reader.read()

      if (done) return

      buffer += decoder.decode(value, { stream: true })

      if (buffer.length > MAX_FRAME_BYTES) throw new Error("chauffeur event frame exceeds 1 MB")

      const frames = buffer.split("\n\n")
      buffer = frames.pop() ?? ""

      for (const frame of frames) {
        const data = frame
          .split("\n")
          .filter((line) => line.startsWith("data: "))
          .map((line) => line.slice(6))
          .join("\n")

        if (data) yield data
      }
    }
  } finally {
    reader.releaseLock()
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

function decodeFrame(data: string): EventFrame | undefined {
  const value: unknown = JSON.parse(data)

  if (!isRecord(value) || typeof value.type !== "string") return undefined
  if (value.type === "heartbeat") return { type: "heartbeat" }
  if (value.type === "subscribed" && isTarget(value.target)) return { type: "subscribed", target: value.target }
  if (value.type !== "event" || !isTarget(value.target) || !Array.isArray(value.reminders)) return undefined

  return { type: "event", target: value.target, reminders: value.reminders as QueuedReminder[] }
}

function isTarget(value: unknown): value is Target {
  return isRecord(value) && value.kind === "opencode-session" && typeof value.id === "string"
}

function isSkillResult(value: unknown): value is SkillResult {
  if (!isRecord(value)) return false

  return typeof value.skill_id === "string"
    && isOneOf(value.status, SKILL_STATUSES)
    && (value.effect === null || isOneOf(value.effect, SKILL_EFFECTS))
    && (typeof value.message === "string" || value.message === null)
    && (typeof value.reminder === "string" || value.reminder === null)
    && Array.isArray(value.missing_evidence)
    && value.missing_evidence.every((item) => typeof item === "string")
    && (value.confidence === null
      || (typeof value.confidence === "number"
        && Number.isFinite(value.confidence)
        && value.confidence >= 0
        && value.confidence <= 1))
    && (value.branch === null || isOneOf(value.branch, SKILL_BRANCHES))
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
