import { expect, test } from "bun:test"
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { pathToFileURL } from "node:url"
import { startupStep } from "../src/daemon.js"
import { handoffSkills, parseSkill } from "../src/handoff.js"
import { bundledBinary, daemonLaunch, packageSkills, PLUGIN_VERSION } from "../src/package.js"

test("the plugin starts, uses, or replaces a daemon by version, and never replaces the user's", () => {
  expect(startupStep(undefined, false)).toBe("start")
  expect(startupStep(undefined, true)).toBe("unavailable")
  expect(startupStep({ version: PLUGIN_VERSION }, false)).toBe("use")
  expect(startupStep({ version: "0.0.1" }, false)).toBe("replace")
  // A daemon from before versioned releases reports none.
  expect(startupStep({}, false)).toBe("replace")
  expect(startupStep({ version: "0.0.1" }, true)).toBe("mismatch")
})

/** A package as npm installs it: the bundle, its skills, and one platform binary. */
function installedPackage() {
  // Real path: module resolution follows macOS's /var -> /private/var link.
  const root = realpathSync(mkdtempSync(join(tmpdir(), "chauffeur-package-")))
  const binary = join(root, "node_modules/@fdcn/chauffeur-darwin-arm64")

  mkdirSync(join(root, "dist"))
  writeFileSync(join(root, "dist/index.js"), "")
  mkdirSync(join(root, "skills/rulebooks"), { recursive: true })
  mkdirSync(join(binary, "bin"), { recursive: true })
  writeFileSync(join(binary, "package.json"), JSON.stringify({ name: "@fdcn/chauffeur-darwin-arm64", version: "1.0.0" }))
  writeFileSync(join(binary, "bin/chauffeur"), "")

  return { root, bundle: pathToFileURL(join(root, "dist/index.js")).href }
}

test("a package install starts its own binary with its own skills; the user's settings win", () => {
  const { root, bundle } = installedPackage()

  try {
    expect(bundledBinary(bundle, "darwin-arm64")).toBe(join(root, "node_modules/@fdcn/chauffeur-darwin-arm64/bin/chauffeur"))
    expect(bundledBinary(bundle, "linux-x64")).toBeUndefined()
    expect(bundledBinary(bundle, "win32-x64")).toBeUndefined()
    expect(packageSkills(bundle)).toBe(join(root, "skills"))

    const own = daemonLaunch({ CHAUFFEUR_BIN: "/opt/chauffeur", CHAUFFEUR_SKILLS_DIR: "/my/skills" }, bundle)

    expect(own).toEqual({ bin: "/opt/chauffeur", env: { CHAUFFEUR_BIN: "/opt/chauffeur", CHAUFFEUR_SKILLS_DIR: "/my/skills" } })
    expect(daemonLaunch({}, bundle).env.CHAUFFEUR_SKILLS_DIR).toBe(join(root, "skills"))
  } finally {
    rmSync(root, { recursive: true })
  }
})

test("a plugin copied into place falls back to chauffeur on PATH and the daemon's own skills", () => {
  const root = mkdtempSync(join(tmpdir(), "chauffeur-copied-"))
  const bundle = pathToFileURL(join(root, "plugins/chauffeur.js")).href

  try {
    expect(daemonLaunch({}, bundle)).toEqual({ bin: "chauffeur", env: {} })
  } finally {
    rmSync(root, { recursive: true })
  }
})

test("the shipped hand-over skills parse with their body as content", () => {
  const skills = handoffSkills(join(import.meta.dir, "../../../skills/handoff"))

  expect(skills.map((skill) => String(skill.id)).toSorted()).toEqual(["jira-cli", "slack-cli", "twitter-cli"])

  for (const skill of skills) {
    expect(skill.description?.length).toBeGreaterThan(20)
    expect(skill.content.startsWith("---")).toBe(false)
  }
})

test("a skill without a name, or with a block-style field, is not registered", () => {
  expect(parseSkill("---\ndescription: x\n---\nbody", "/s/SKILL.md")).toBeUndefined()
  expect(parseSkill("---\nname: s\ndescription: >-\n  folded\n---\nbody", "/s/SKILL.md")).toBeUndefined()
  expect(parseSkill("no frontmatter", "/s/SKILL.md")).toBeUndefined()
  expect(parseSkill("---\nname: s\ndescription: \"quoted\"\n---\n\nbody\n", "/s/SKILL.md")).toMatchObject({ id: "s", description: "quoted", content: "body" })
})
