import { expect, test } from "bun:test"
import { describe, messages } from "../src/act.ts"
import { NUDGE } from "./stand-in.ts"

const with_ = (payload: Record<string, unknown>) => ({
  ...NUDGE,
  payload: { ...NUDGE.payload, ...payload }
})

test("a nudge now is steered into the running turn, under its label", () => {
  const { messages: found, skipped } = messages([NUDGE])

  expect(found).toEqual([
    {
      rule: "conventional-commits",
      text: "Amend it to `type(scope): description`.",
      description: "Chauffeur · conventional-commits: commit message isn't conventional",
      delivery: "steer",
      resume: false
    }
  ])
  expect(skipped).toEqual([])
})

test("next waits for the next turn, and wake starts one", () => {
  const [next] = messages([with_({ delivery: "next" })]).messages
  const [wake] = messages([with_({ delivery: "wake" })]).messages

  expect([next?.delivery, next?.resume]).toEqual(["queue", false])
  expect([wake?.delivery, wake?.resume]).toEqual(["queue", true])
})

test("text parts follow one another, and an unlabelled one shows its rule", () => {
  const parts = [{ text: "One." }, { skill: "slack-cli" }, { text: "Two." }]
  const [message] = messages([with_({ parts, label: undefined })]).messages

  expect(message?.text).toBe("One.\n\nTwo.")
  expect(message?.description).toBe("Chauffeur · conventional-commits")
  expect(describe("r", "says why")).toBe("Chauffeur · r: says why")
})

test("other actions and payloads with nothing to deliver are skipped", () => {
  const compact = { rule: "renewal", action: "session.compact" }
  const later = with_({ delivery: "later" })
  const inherited = with_({ delivery: "toString" })
  const empty = with_({ parts: [{ text: "" }] })
  const bare = { rule: "r", action: "message.append" }
  const all = [compact, later, inherited, empty, bare]

  const { messages: found, skipped } = messages(all)

  expect(found).toEqual([])
  expect(skipped).toEqual(all)
})
