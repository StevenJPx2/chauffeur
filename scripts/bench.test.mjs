import assert from "node:assert/strict"
import { test } from "node:test"

import { parse } from "./bench.mjs"

const DEFAULTS = ["-benchtime", "1s", "-count", "6", "-benchmem"]

test("with no arguments, this tree alone with the default settings", () => {
  assert.deepEqual(parse([]), { base: undefined, bench: DEFAULTS })
})

test("a base commit, and arguments after -- passed to every benchmark", () => {
  assert.deepEqual(parse(["v2~3", "--", "-bench", "Redact"]), {
    base: "v2~3",
    bench: [...DEFAULTS, "-bench", "Redact"]
  })
})

test("more than one base commit is refused", () => {
  assert.throws(() => parse(["a", "b"]), /at most one base commit/)
})
