import { expect, test } from "bun:test"
import type { Asked } from "../src/judge/index.ts"
import type { Navigated } from "../src/client/index.ts"

// Checked by `tsc`: each @ts-expect-error fails the typecheck when the line
// it marks compiles, so these are compile-time tests.
function outcomes(done: Asked<"yes" | "no">): string {
  if (done.ok && done.outcome === "yes") return "yes"

  if (done.ok && done.outcome === "unsure") return "unsure"

  // @ts-expect-error "maybe" is not one of a noul's outcomes.
  if (done.ok && done.outcome === "maybe") return "never"

  // @ts-expect-error an answer that failed has no outcome.
  if (!done.ok) return done.outcome

  return "no"
}

function results(
  event: Navigated<{ on: "x" }>,
  strategy: Navigated<{ strategy: [] }>
) {
  // @ts-expect-error an event's result has intents, not a verdict.
  event.verdict

  // @ts-expect-error a strategy's result has a verdict, not intents.
  strategy.intents

  return [event.intents, strategy.verdict]
}

test("the compile-time checks above are part of the typecheck", () => {
  expect(typeof outcomes).toBe("function")
  expect(typeof results).toBe("function")
})
