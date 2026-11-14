import type { Feature } from "../feature.ts"
import { navigate as run } from "../wasm/chauffeur_playground.js"

/** sourcefed's triage rule, as its installer writes it. */
const triage = {
  on: "sourcefed.event",
  judges: { quick: { tier: "system-one", confidence: 0.7 } },
  strategy: [
    {
      id: "screen",
      judge: "quick",
      ask: {
        outcomes: ["yes", "no"],
        evidence: ["event", "thread", "monitor"],
        instructions: "Should the agent be shown this event now?",
      },
    },
  ],
  then: [{ if: { screen: "no" }, withhold: "event" }],
}

const sourcefed = (event: string) => ({
  protocol: 1,
  deadline_ms: 3000,
  on: "sourcefed.event",
  evidence: { event, thread: "PR #42: feat(cli): add navigate", monitor: "PR #42 in wrkhse/chauffeur" },
})

const finished = {
  protocol: 1,
  judges: { quick: { tier: "system-one", confidence: 0.7 } },
  strategy: [
    {
      id: "done",
      judge: "quick",
      ask: { outcomes: ["yes", "no"], evidence: ["log"], instructions: "Is the migration finished?" },
    },
  ],
  evidence: { log: "migrated 42 of 42 tables" },
}

/** Requests with the rules that answer them and each step's stand-in reply. */
const examples: Readonly<Record<string, unknown>> = {
  "sourcefed: noise is withheld": {
    request: sourcefed("thanks, looks good 👍"),
    rules: { "sourcefed/triage": triage },
    answers: { screen: { yes: 0.06, no: 0.94 } },
  },
  "sourcefed: a CI failure is delivered": {
    request: sourcefed("CI failed on main: 3 tests"),
    rules: { "sourcefed/triage": triage },
    answers: { screen: { yes: 0.95, no: 0.05 } },
  },
  "sourcefed: unsure is delivered": {
    request: sourcefed("which doc?"),
    rules: { "sourcefed/triage": triage },
    answers: { screen: { yes: 0.48, no: 0.52 } },
  },
  "sourcefed: a timed-out judge is delivered": {
    request: sourcefed("deploy is on fire"),
    rules: { "sourcefed/triage": triage },
    answers: { screen: { failed: "timeout" } },
  },
  "An inline strategy's verdict": { request: finished, answers: { done: { yes: 0.91, no: 0.09 } } },
  "Over the deadline limit": { request: { ...sourcefed("x"), deadline_ms: 60000 }, rules: {} },
  "Another protocol": { request: { ...sourcefed("x"), protocol: 2 }, rules: {} },
}

type Navigated =
  | { events: ReadonlyArray<Record<string, unknown>>; result: Record<string, unknown>; refused?: ReadonlyArray<string> }
  | { rejected: { code: number; message: string; data: Record<string, unknown> } }
  | { error: string }

type NavigateView = {
  readonly $template: string
  readonly names: ReadonlyArray<string>
  example: string
  source: string
  result: Navigated | undefined
  load(name: string): void
  update(source: string): void
  pretty(value: unknown): string
}

const first = Object.keys(examples)[0] ?? ""
const show = (value: unknown): string => JSON.stringify(value, null, 2)

/** A request, rule files and stand-in replies in; the events and intents, the verdict, or the rejection out. */
export const navigate: Feature = {
  id: "navigate",
  title: "Navigate",
  view: (): NavigateView => {
    const view: NavigateView = {
      $template: "#navigate",
      names: Object.keys(examples),
      example: first,
      source: show(examples[first]),
      result: undefined,
      load(name) {
        this.example = name
        this.update(show(examples[name]))
      },
      update(source) {
        this.source = source
        try {
          JSON.parse(source)
          this.result = JSON.parse(run(source)) as Navigated
        } catch (error) {
          this.result = { error: error instanceof Error ? error.message : String(error) }
        }
      },
      pretty: show,
    }

    view.update(view.source)
    return view
  },
}
