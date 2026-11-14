import { connect, navigate, type Chauffeur } from "../client/navigate.ts"
import {
  NavigateError,
  type Blocks,
  type Failure,
  type JudgeSpec,
  type Reason,
  type StrategyRequest,
  type Tier
} from "../client/wire.ts"

/** A named judge: a tier, the model asked first, and how sure an answer must be. */
export type Judge = JudgeSpec

/**
 * A judge for `ask`. `confidence` is how likely the top outcome must be,
 * from 0 to 1, or one bar per outcome; without it the likeliest settles.
 */
export function judge(spec: {
  readonly tier: Tier
  readonly model?: string
  readonly confidence?: number | Readonly<Record<string, number>>
}): Judge {
  const bars =
    typeof spec.confidence === "number"
      ? [spec.confidence]
      : Object.values(spec.confidence ?? {})

  for (const bar of bars) {
    if (!(bar >= 0 && bar <= 1))
      throw new RangeError(`confidence ${bar} is not from 0 to 1`)
  }

  return spec
}

/** A question: its instructions and its outcomes, in order. */
export type Question<O extends string> = {
  readonly instructions: string
  readonly outcomes: readonly O[]
}

/** A yes/no question, using Jev's name for it. */
export function noul(instructions: string): Question<"yes" | "no"> {
  return { instructions, outcomes: ["yes", "no"] }
}

/** An answer: one of the question's outcomes or `unsure`, and each outcome's probability. */
export type Answered<O extends string> = {
  readonly ok: true
  readonly outcome: O | "unsure"
  readonly p: Readonly<Record<O, number>>
  readonly level: number
  readonly model: string
  readonly placeholder: boolean
}

/** No answer: the judge failed, or the request was rejected or never reached the daemon. */
export type Unanswered = { readonly ok: false; readonly reason: Failure | Reason }

export type Asked<O extends string> = Answered<O> | Unanswered

/** How `ask` is sent. */
export type AskOptions = {
  readonly judge: Judge
  readonly evidence?: Blocks
  readonly signal?: AbortSignal
  readonly deadlineMs?: number
  /** A client kept with `connect()`; without one, a connection of its own. */
  readonly client?: Chauffeur
}

/**
 * Asks `question` of `options.judge` over `options.evidence`, as a one-step
 * inline strategy. Never rejects: a failure, a rejection, no daemon and an
 * abort all answer `{ ok: false, reason }`.
 */
export async function ask<O extends string>(
  question: Question<O>,
  options: AskOptions
): Promise<Asked<O>> {
  const evidence = options.evidence ?? {}
  const request: StrategyRequest = {
    judges: { asked: options.judge },
    strategy: [
      {
        id: "ask",
        judge: "asked",
        ask: { ...question, evidence: Object.keys(evidence) }
      }
    ],
    evidence,
    ...(options.deadlineMs === undefined ? {} : { deadline_ms: options.deadlineMs })
  }
  const signal = options.signal ? { signal: options.signal } : {}

  try {
    const { verdict } = options.client
      ? await options.client.navigate(request, signal)
      : await navigate(request, signal)
    const settled = verdict.ask

    if (!settled) return { ok: false, reason: "invalid" }

    if ("failed" in settled) return { ok: false, reason: settled.failed }

    return { ok: true, ...settled.answer } as Answered<O>
  } catch (error) {
    if (error instanceof NavigateError) return { ok: false, reason: error.reason }

    return {
      ok: false,
      reason: options.signal?.aborted ? "cancelled" : "unavailable"
    }
  }
}

export { connect, type Chauffeur }
