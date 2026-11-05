import { expect, test } from "bun:test"
import { mkdtempSync, readFileSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { fileLog, logPath } from "../src/log.ts"

test("the log is in the installation's state folder", () => {
  expect(logPath("chauffeur-next", { XDG_STATE_HOME: "/state" })).toBe(
    "/state/chauffeur-next/opencode.log"
  )

  expect(logPath("chauffeur-next", { HOME: "/home/me" })).toBe(
    "/home/me/.local/state/chauffeur-next/opencode.log"
  )
})

test("each line carries the time, and the folder is made", () => {
  const folder = mkdtempSync(join(tmpdir(), "cho-log-"))
  const path = join(folder, "chauffeur-next", "opencode.log")
  const log = fileLog(path, () => new Date("2026-11-05T02:00:00Z"))

  try {
    log("chauffeur: nothing sent")
    log("chauffeur: again")

    expect(readFileSync(path, "utf8")).toBe(
      "2026-11-05T02:00:00.000Z chauffeur: nothing sent\n" +
        "2026-11-05T02:00:00.000Z chauffeur: again\n"
    )
  } finally {
    rmSync(folder, { recursive: true })
  }
})

test("a log that cannot be written is let go", () => {
  expect(() => fileLog("/dev/null/opencode.log")("lost")).not.toThrow()
})
