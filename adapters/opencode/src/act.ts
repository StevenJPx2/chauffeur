import type { Action } from "./daemon.ts"

/** One message for OpenCode's `session.synthetic`, from a `message.append`. */
export type Message = {
  readonly rule: string
  readonly text: string
  readonly description: string
  readonly delivery: "steer" | "queue"
  readonly resume: boolean
}

export type Planned = {
  readonly messages: readonly Message[]
  readonly skipped: readonly Action[]
}

type Payload = { parts?: unknown; delivery?: unknown; label?: unknown }

const DELIVERIES: Readonly<Record<string, Pick<Message, "delivery" | "resume">>> = {
  now: { delivery: "steer", resume: false },
  next: { delivery: "queue", resume: false },
  wake: { delivery: "queue", resume: true }
}

/**
 * Each `message.append` as one message: its text parts after one another,
 * described as `Chauffeur · <rule>: <label>`. `now` steers it into the
 * running turn, `next` queues it for the next turn, and `wake` queues it
 * and starts a turn when the agent is idle. Anything else is skipped.
 */
export function messages(actions: readonly Action[]): Planned {
  const found: Message[] = []
  const skipped: Action[] = []

  for (const action of actions) {
    const message = toMessage(action)

    if (message) {
      found.push(message)
    } else {
      skipped.push(action)
    }
  }

  return { messages: found, skipped }
}

function toMessage(action: Action): Message | undefined {
  if (action.action !== "message.append") return undefined

  const payload = (action.payload ?? {}) as Payload
  const delivery = typeof payload.delivery === "string" ? payload.delivery : ""
  const how = Object.hasOwn(DELIVERIES, delivery) ? DELIVERIES[delivery] : undefined
  const texts = Array.isArray(payload.parts) ? payload.parts.flatMap(textOf) : []

  if (!how || texts.length === 0) return undefined

  const rule = action.rule ?? "chauffeur"
  const label = typeof payload.label === "string" ? payload.label : ""

  return {
    rule,
    text: texts.join("\n\n"),
    description: describe(rule, label),
    ...how
  }
}

function textOf(part: unknown): string[] {
  const text = (part as { text?: unknown } | null)?.text

  return typeof text === "string" && text !== "" ? [text] : []
}

/** What OpenCode shows on the message's row. */
export function describe(rule: string, label: string): string {
  return label === "" ? `Chauffeur · ${rule}` : `Chauffeur · ${rule}: ${label}`
}
