import { expect, test } from "bun:test"
import { apply } from "../src/act.ts"
import { NUDGE } from "./stand-in.ts"

const TEXT = "Use Conventional Commits, such as `feat(scope): …`."

test("a nudge follows a text result after a blank line", () => {
  const { result, skipped } = apply({ content: "[v2 3f2c1a0] fixed stuff" }, [NUDGE])

  expect(result.content).toBe(`[v2 3f2c1a0] fixed stuff\n\n${TEXT}`)
  expect(skipped).toEqual([])
})

test("a nudge is a text item after a list result's items", () => {
  const image = { type: "file" as const, uri: "file:///a.png", mime: "image/png" }
  const { result } = apply({ content: [image] }, [NUDGE])

  expect(result.content).toEqual([image, { type: "text", text: `\n\n${TEXT}` }])
})

test("a nudge follows the output when the result has no content", () => {
  const { result } = apply({ output: "done", metadata: { exit: 0 } }, [NUDGE])

  expect(result).toEqual({
    output: "done",
    metadata: { exit: 0 },
    content: `done\n\n${TEXT}`
  })
})

test("no actions leave the result as it was", () => {
  const before = { content: "ok" }

  expect(apply(before, []).result).toBe(before)
})

test("other actions and deliveries are skipped", () => {
  const next = { ...NUDGE, payload: { ...NUDGE.payload, delivery: "next" } }
  const compact = { rule: "renewal", action: "session.compact" }
  const { result, skipped } = apply({ content: "ok" }, [next, compact])

  expect(result.content).toBe("ok")
  expect(skipped).toEqual([next, compact])
})
