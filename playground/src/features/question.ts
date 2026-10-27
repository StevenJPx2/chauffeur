import type { Feature } from "../feature.ts"
import { settle } from "../wasm/chauffeur_playground.js"

type Input = {
  outcomes: ReadonlyArray<string>
  p: Readonly<Record<string, number>>
  confidence?: number | Readonly<Record<string, number>>
}

/** The plan's confidence examples. */
const examples: Readonly<Record<string, Input>> = {
  '"fixed stuff", bar 0.7': { outcomes: ["yes", "no"], p: { yes: 0.94, no: 0.06 }, confidence: 0.7 },
  '"Update parser.rs", bar 0.7': { outcomes: ["yes", "no"], p: { yes: 0.58, no: 0.42 }, confidence: 0.7 },
  '"rm -rf build/", per outcome': { outcomes: ["yes", "no"], p: { yes: 0.71, no: 0.29 }, confidence: { yes: 0.6, no: 0.95 } },
  '"git clean -fd", per outcome': { outcomes: ["yes", "no"], p: { yes: 0.12, no: 0.88 }, confidence: { yes: 0.6, no: 0.95 } },
  "A scale, bar 0.7": { outcomes: ["low", "medium", "high"], p: { low: 0.12, medium: 0.45, high: 0.43 }, confidence: 0.7 },
  "A tie, no bar": { outcomes: ["yes", "no"], p: { yes: 0.5, no: 0.5 } },
  "A reply missing an outcome": { outcomes: ["yes", "no"], p: { yes: 0.9 }, confidence: 0.7 },
}

type Settled =
  | { answer: { outcome: string; p: ReadonlyArray<{ outcome: string; p: number }>; level: number } }
  | { failed: string }
  | { error: string }

type QuestionView = {
  readonly $template: string
  readonly names: ReadonlyArray<string>
  example: string
  source: string
  result: Settled | undefined
  load(name: string): void
  update(source: string): void
}

const first = Object.keys(examples)[0] ?? ""
const show = (input: Input | undefined): string => JSON.stringify(input, null, 2)

/** Outcomes, probabilities and a confidence in; the settled outcome, `unsure` or a failure out. */
export const question: Feature = {
  id: "question",
  title: "Question",
  view: (): QuestionView => {
    const view: QuestionView = {
      $template: "#question",
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
          this.result = JSON.parse(settle(source)) as Settled
        } catch (error) {
          this.result = { error: error instanceof Error ? error.message : String(error) }
        }
      },
    }

    view.update(view.source)
    return view
  },
}
