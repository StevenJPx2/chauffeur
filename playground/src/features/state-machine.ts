import type { Feature } from "../feature.ts"
import { Session } from "../wasm/chauffeur_playground.js"

type GateJson = { mode: "any" | "all"; sources: ReadonlyArray<string>; on: ReadonlyArray<string> }
type StepJson = { id: string; judge: string; ask: string; gates?: ReadonlyArray<GateJson> }
type PlanJson = { steps: ReadonlyArray<StepJson> }

/** `person` steps are answered on this page; `keywords` is a rule judge in Rust. */
const step = (id: string, ask: string, ...gates: ReadonlyArray<GateJson>): StepJson => ({
  id,
  judge: "person",
  ask,
  ...(gates.length === 0 ? {} : { gates }),
})
const any = (sources: ReadonlyArray<string>, ...on: ReadonlyArray<string>): GateJson => ({ mode: "any", sources, on })
const all = (sources: ReadonlyArray<string>, ...on: ReadonlyArray<string>): GateJson => ({ mode: "all", sources, on })

/** The strategies the core's builders compose, written out as the plans they lower to. */
const examples: Readonly<Record<string, PlanJson>> = {
  "Rule, then person": {
    steps: [
      { id: "screen", judge: "keywords", ask: "Is work left? The tests pass." },
      step("review", "Look closer: is work left?", any(["screen"], "unsure", "failed")),
    ],
  },
  "Screen, then review": {
    steps: [
      step("screen", "Is work left?"),
      step("review", "Look closer: is work left?", any(["screen"], "unsure", "failed")),
    ],
  },
  Single: { steps: [step("screen", "Is work left?")] },
  Chain: {
    steps: [
      step("a", "First check"),
      step("b", "Second check", any(["a"], "yes")),
      step("c", "Third check", any(["b"], "yes")),
    ],
  },
  "Fan out": {
    steps: [step("slack", "Use the Slack skill?"), step("jira", "Use the Jira skill?"), step("memory", "Use memory?")],
  },
  Fallback: {
    steps: [step("primary", "Is work left?"), step("fallback", "Is work left? (fallback)", any(["primary"], "failed"))],
  },
  "Fan out, then review": {
    steps: [
      step("slack", "Use the Slack skill?"),
      step("jira", "Use the Jira skill?"),
      step("memory", "Use memory?"),
      step("review", "Confirm the chosen skills", any(["slack", "jira", "memory"], "yes")),
    ],
  },
  "All must agree": {
    steps: [step("a", "Check A"), step("b", "Check B"), step("both", "Both said yes", all(["a", "b"], "yes"))],
  },
}

const first = Object.keys(examples)[0] ?? ""
const show = (plan: PlanJson | undefined): string => JSON.stringify(plan, null, 2)

const strings = (value: unknown, where: string): ReadonlyArray<string> => {
  if (!Array.isArray(value) || !value.every((item) => typeof item === "string")) {
    throw new Error(`${where} must be a list of strings`)
  }
  return value
}

/** Check the JSON's shape; the core judges whether the plan itself is valid. */
function parse(source: string): PlanJson {
  const plan: unknown = JSON.parse(source)
  if (typeof plan !== "object" || plan === null || !("steps" in plan) || !Array.isArray(plan.steps)) {
    throw new Error("a plan is an object with a `steps` list")
  }
  return {
    steps: plan.steps.map((raw: unknown, index): StepJson => {
      const where = `step ${index + 1}`
      if (typeof raw !== "object" || raw === null) throw new Error(`${where} must be an object`)
      const { id, judge, ask, gates = [] } = raw as Record<string, unknown>
      if (typeof id !== "string" || typeof judge !== "string" || typeof ask !== "string") {
        throw new Error(`${where} needs string \`id\`, \`judge\` and \`ask\``)
      }
      if (!Array.isArray(gates)) throw new Error(`${where}: \`gates\` must be a list`)
      return {
        id,
        judge,
        ask,
        gates: gates.map((gate: Record<string, unknown>): GateJson => {
          if (gate.mode !== "any" && gate.mode !== "all") throw new Error(`${where}: a gate's mode is "any" or "all"`)
          return { mode: gate.mode, sources: strings(gate.sources, `${where}: sources`), on: strings(gate.on, `${where}: on`) }
        }),
      }
    }),
  }
}

function build(plan: PlanJson): Session {
  const session = new Session()
  for (const { id, judge, ask, gates = [] } of plan.steps) {
    session.add_step(id, judge, ask)
    for (const gate of gates) session.add_gate(gate.mode, [...gate.sources], [...gate.on])
  }
  return session
}

/** One asked step. A rule judge's step has its `ruling`; a person's is empty. */
type Question = { id: string; judge: string; request: string; ruling: string; answer: string }
type Line = { id: string; resolution: string }

type StateMachine = {
  readonly $template: string
  readonly names: ReadonlyArray<string>
  example: string
  source: string
  error: string
  round: number
  questions: Question[]
  events: string[]
  verdict: Line[]
  load(name: string): void
  start(): void
  answer(): void
  fail(): void
  /** Run one action against the core, then show its state or its error. */
  run(act: () => void): void
}

/** A plan in, the core's rounds, events and verdict out. Rule judges answer their steps; the rest are picked by hand. */
export const stateMachine: Feature = {
  id: "state-machine",
  title: "State machine",
  view: (): StateMachine => {
    // The wasm object stays outside the reactive scope; the view copies what it shows.
    let session: Session | undefined

    const view: StateMachine = {
      $template: "#state-machine",
      names: Object.keys(examples),
      example: first,
      source: show(examples[first]),
      error: "",
      round: 0,
      questions: [],
      events: [],
      verdict: [],
      load(name) {
        this.example = name
        this.source = show(examples[name])
        this.start()
      },
      start() {
        session?.free()
        session = undefined
        this.run(() => {
          session = build(parse(this.source))
          session.start()
        })
      },
      answer() {
        const byHand = this.questions.filter((question) => question.ruling === "")
        this.run(() => {
          session?.answer(
            byHand.map((question) => question.id),
            byHand.map((question) => question.answer),
          )
        })
      },
      fail() {
        this.run(() => session?.fail_round())
      },
      run(act) {
        try {
          act()
          this.error = ""
        } catch (error) {
          this.error = error instanceof Error ? error.message : String(error)
        }
        const asked = session?.asked() ?? []
        const judges = session?.judges() ?? []
        const requests = session?.requests() ?? []
        const rulings = session?.rulings() ?? []
        const resolutions = session?.verdict_resolutions() ?? []
        this.round = session?.round() ?? 0
        this.questions = asked.map((id, index) => ({
          id,
          judge: judges[index] ?? "",
          request: requests[index] ?? "",
          ruling: rulings[index] ?? "",
          answer: "yes",
        }))
        this.events = session?.events() ?? []
        this.verdict = (session?.verdict_steps() ?? []).map((id, index) => ({ id, resolution: resolutions[index] ?? "" }))
      },
    }

    view.start()
    return view
  },
}
