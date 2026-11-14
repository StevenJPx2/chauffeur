/** What goes over the socket for `chauffeur.navigate`, as the protocol crate's schema defines it. */

/** The protocol versions this client speaks. */
export const PROTOCOL = 1

/** Named blocks of text a judge may see, or candidates a rule may rank. */
export type Blocks = Readonly<Record<string, string>>

/** A tier a step's judge can name. */
export type Tier = "facts" | "system-one" | "system-two" | "person"

/** A named judge: its tier, the model asked first, and how sure an answer must be. */
export type JudgeSpec = {
  readonly tier: Tier
  readonly model?: string
  readonly confidence?: number | Readonly<Record<string, number>>
}

/** An outcome, or one with a description the model sees. */
export type Label = string | { readonly value: string; readonly description: string }

/** A question: instructions, its outcomes, and the evidence blocks the judge sees. */
export type Ask = {
  readonly instructions: string
  readonly outcomes: readonly Label[]
  readonly evidence?: readonly string[]
}

/** One step: its ID, the judge that answers it, and its question or an ask's name. */
export type Step = {
  readonly id: string
  readonly judge: string
  readonly ask: Ask | string
}

type Common = {
  readonly evidence?: Blocks
  readonly candidates?: Blocks
  /** How long the rules may take, at most 30 000; the daemon's default is 3 000. */
  readonly deadline_ms?: number
}

/** An event every rule whose `on` and `when` match answers with its intents. */
export type EventRequest = Common & { readonly on: string }

/** One strategy, run alone, which answers with its verdict. */
export type StrategyRequest = Common & {
  readonly strategy: readonly Step[]
  readonly judges?: Readonly<Record<string, JudgeSpec>>
  readonly asks?: Readonly<Record<string, Ask>>
}

export type NavigateRequest = EventRequest | StrategyRequest

/** Why a judge gave no answer. */
export type Failure =
  | "unavailable"
  | "rate_limited"
  | "timeout"
  | "invalid"
  | "deadline"

/** A judge's answer: the outcome, or `unsure`, every outcome's probability, the level, and who answered. */
export type Answer = {
  readonly outcome: string
  readonly p: Readonly<Record<string, number>>
  readonly level: number
  readonly model: string
  readonly placeholder: boolean
}

/** A step's result: an answer, or why it failed. */
export type Settled = { readonly answer: Answer } | { readonly failed: Failure }

/** What a rule asks its caller to do, such as `{ rule: "sourcefed/triage", withhold: "event" }`. */
export type Intent = { readonly rule: string; readonly [intent: string]: unknown }

export type Intents = { readonly intents: readonly Intent[] }
export type Verdict = { readonly verdict: Readonly<Record<string, Settled>> }

/** What a request answers with: intents for an event, a verdict for a strategy. */
export type Navigated<R extends NavigateRequest> = R extends StrategyRequest
  ? Verdict
  : Intents

/** One event of a navigation as it runs. */
export type NavigationEvent =
  | { readonly event: "rule_matched"; readonly rule: string }
  | {
      readonly event: "round_ready"
      readonly rule?: string
      readonly round: number
      readonly steps: readonly string[]
    }
  | ({
      readonly event: "step_settled"
      readonly rule?: string
      readonly step: string
    } & Settled)

/** Why a navigation gave no intents or verdict. `unavailable` is also no daemon, or another protocol. */
export type Reason =
  | "invalid"
  | "limit_exceeded"
  | "busy"
  | "deadline"
  | "judge_unavailable"
  | "cancelled"
  | "unavailable"

/** A navigation that was rejected, or could not reach the daemon. */
export class NavigateError extends Error {
  override readonly name = "NavigateError"

  constructor(
    readonly reason: Reason,
    message: string,
    readonly limit?: string,
    readonly max?: number
  ) {
    super(message)
  }
}

/** The error the daemon answered with, as a `NavigateError`. */
export function rejected(error: {
  readonly message?: unknown
  readonly data?: {
    readonly reason?: unknown
    readonly limit?: unknown
    readonly max?: unknown
  }
}): NavigateError {
  const data = error.data ?? {}
  const message =
    typeof error.message === "string"
      ? error.message
      : "the daemon refused the request"
  const reason = typeof data.reason === "string" ? reasonOf(data.reason) : "invalid"
  const limit = typeof data.limit === "string" ? data.limit : undefined
  const max = typeof data.max === "number" ? data.max : undefined

  return new NavigateError(reason, message, limit, max)
}

const REASONS = new Set<string>([
  "invalid",
  "limit_exceeded",
  "busy",
  "deadline",
  "judge_unavailable",
  "cancelled",
  "unavailable"
])

/** The daemon's reason as one this client knows; another protocol is `unavailable`. */
function reasonOf(reason: string): Reason {
  if (reason === "unsupported_protocol") return "unavailable"

  return REASONS.has(reason) ? (reason as Reason) : "invalid"
}
