import { expect, test } from "bun:test"
import { clip, MAX_OUTPUT_BYTES, text, toolAfter } from "../src/event.ts"

const commit = {
  id: "toolu_1",
  tool: "shell",
  sessionID: "ses_1",
  input: { command: "git commit -m 'fixed stuff'" }
}

test("a completed call is a tool.after event with its output", () => {
  const event = toolAfter({
    ...commit,
    status: "completed",
    result: { content: "[v2 3f2c1a0] fixed stuff" }
  })

  expect(event).toEqual({
    event: "tool.after",
    session: "ses_1",
    tool: "shell",
    input: { command: "git commit -m 'fixed stuff'" },
    output: "[v2 3f2c1a0] fixed stuff"
  })
})

test("a failed call is not an event", () => {
  expect(toolAfter({ ...commit, status: "error" })).toBeUndefined()
})

test("the output is the result's text content, else its output", () => {
  const items = [
    { type: "text" as const, text: "one" },
    { type: "file" as const, uri: "file:///a.png", mime: "image/png" },
    { type: "text" as const, text: "two" }
  ]

  expect(text({ content: items })).toBe("one\ntwo")
  expect(text({ output: { exit: 0 } })).toBe('{"exit":0}')
  expect(text({ output: "plain" })).toBe("plain")
  expect(text({})).toBe("")
})

test("the output stops at 64 KiB, between characters", () => {
  const long = "é".repeat(MAX_OUTPUT_BYTES)
  const clipped = clip(long, MAX_OUTPUT_BYTES)

  expect(new TextEncoder().encode(clipped).length).toBe(MAX_OUTPUT_BYTES)
  expect(clipped.endsWith("é")).toBe(true)
  expect(clip("é".repeat(3), 5)).toBe("éé")
  expect(clip("short", MAX_OUTPUT_BYTES)).toBe("short")
})
