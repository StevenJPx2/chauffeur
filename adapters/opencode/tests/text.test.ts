import { expect, test } from "bun:test"
import { clipEndsBytes } from "../src/text.js"

test("clipEndsBytes keeps short text whole", () => {
  expect(clipEndsBytes("Done.", 2_048)).toBe("Done.")
})

test("clipEndsBytes keeps both ends within the byte bound, weighted to the tail", () => {
  const value = `${"a".repeat(3_000)}## Open questions\n- Floppy or Yamtrack?\n\n${"Sources: ".repeat(60)}`
  const clipped = clipEndsBytes(value, 2_048)

  expect(Buffer.byteLength(clipped, "utf8")).toBeLessThanOrEqual(2_048)
  expect(clipped.startsWith("aaa")).toBe(true)
  expect(clipped).toContain("Floppy or Yamtrack?")
  expect(clipped.endsWith("Sources: ")).toBe(true)
})

test("clipEndsBytes never splits a code point", () => {
  const clipped = clipEndsBytes("😀".repeat(1_000), 2_048)

  expect(Buffer.byteLength(clipped, "utf8")).toBeLessThanOrEqual(2_048)
  expect(clipped.replaceAll("😀", "").replace("…", "")).toBe("")
})
