import { expect, test } from "bun:test"
import { clippedSkills } from "../src/clipped-skills.js"

test("only descriptions past the signal's 400 bytes are reported, with where the cut falls", () => {
  const long = `${"a".repeat(390)} keep this. Never seen past here.`
  // Multi-byte characters count by bytes, not characters.
  const accented = "é".repeat(201)

  const clipped = clippedSkills([
    { id: "short", description: "Read the docs." },
    { id: "missing" },
    { id: "long", description: long },
    { id: "accented", description: accented },
  ])

  expect(clipped.map((skill) => [skill.id, skill.bytes])).toEqual([["long", 423], ["accented", 402]])
  expect(clipped[0]?.endsWith.endsWith("a keep this")).toBe(true)
  expect(clipped[0]?.endsWith).not.toContain("Never")
})
