// node --test scripts/

import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"
import { mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, unlinkSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { test } from "node:test"
import { pathToFileURL } from "node:url"
import { formula, parseSums } from "./homebrew.mjs"
import { sums, tarball, version } from "./package.mjs"

/** A checkout with just what packaging reads, at `cargo` and `plugin` versions. */
function checkout(cargo = "0.2.0", plugin = cargo) {
  const root = mkdtempSync(join(tmpdir(), "chauffeur-release-"))

  writeFileSync(join(root, "Cargo.toml"), `[workspace]\n\n[workspace.package]\nversion = "${cargo}"\nlicense = "MIT"\n`)
  mkdirSync(join(root, "adapters/opencode/dist"), { recursive: true })
  writeFileSync(join(root, "adapters/opencode/package.json"), JSON.stringify({ version: plugin }))
  writeFileSync(join(root, "adapters/opencode/dist/index.mjs"), "export default {}\n")
  mkdirSync(join(root, "skills/rulebooks"), { recursive: true })
  writeFileSync(join(root, "skills/rulebooks/goal.json"), "{}\n")
  writeFileSync(join(root, "LICENSE"), "MIT\n")
  writeFileSync(join(root, "chauffeur"), "#!/bin/sh\n")

  return root
}

test("Cargo and the plugin must ship one version", () => {
  const same = checkout("0.2.0")
  const split = checkout("0.2.0", "0.1.9")

  try {
    assert.equal(version(same), "0.2.0")
    assert.throws(() => version(split), /0\.2\.0 but the plugin is 0\.1\.9/)
  } finally {
    rmSync(same, { recursive: true })
    rmSync(split, { recursive: true })
  }
})

test("a tarball lays out the binary, the bundle beside its skills, and the license", () => {
  const root = checkout()
  const out = join(root, "out")

  try {
    const path = tarball({ target: "darwin-arm64", binary: join(root, "chauffeur"), out, root })
    const listed = execFileSync("tar", ["-tzf", path], { encoding: "utf8" }).split("\n")

    assert.ok(path.endsWith("chauffeur-0.2.0-darwin-arm64.tar.gz"))
    for (const entry of ["bin/chauffeur", "share/chauffeur/dist/index.mjs", "share/chauffeur/skills/rulebooks/goal.json", "share/chauffeur/LICENSE"]) {
      assert.ok(listed.includes(entry), `${entry} in ${listed.join(", ")}`)
    }
    assert.throws(() => tarball({ target: "win32-x64", binary: join(root, "chauffeur"), out, root }), /unknown target/)

    const text = sums(out)

    assert.match(text, /^[0-9a-f]{64} {2}chauffeur-0\.2\.0-darwin-arm64\.tar\.gz\n$/)
    assert.equal(readFileSync(join(out, "SHA256SUMS"), "utf8"), text)
  } finally {
    rmSync(root, { recursive: true })
  }
})

const TARGETS = ["darwin-arm64", "darwin-x64", "linux-arm64", "linux-x64"]

function fixtureSums(release) {
  return TARGETS.map((target, at) => `${String(at).repeat(64)}  chauffeur-${release}-${target}.tar.gz`).join("\n")
}

test("the formula names every target's tarball and checksum", () => {
  const ruby = formula("0.2.0", parseSums(fixtureSums("0.2.0")))

  for (const [at, target] of TARGETS.entries()) {
    assert.ok(ruby.includes(`releases/download/v0.2.0/chauffeur-0.2.0-${target}.tar.gz"\n      sha256 "${String(at).repeat(64)}"`), target)
  }
  assert.match(ruby, /on_macos do\n {4}on_arm do/)
  assert.match(ruby, /on_linux do\n {4}on_arm do/)
  assert.match(ruby, /CHAUFFEUR_MANAGED: {4}"brew"/)
  assert.match(ruby, /version "0\.2\.0"/)
  assert.match(ruby, /await import\("#\{opt_pkgshare\}\/dist\/index\.mjs"\)/)
  assert.ok(ruby.indexOf("process.env.CHAUFFEUR_BIN") < ruby.indexOf("await import("))
})

test("a formula needs a stable version and every target", () => {
  assert.throws(() => formula("0.2.0-rc.1", parseSums(fixtureSums("0.2.0-rc.1"))), /stable version/)

  const missing = parseSums(fixtureSums("0.2.0").split("\n").slice(1).join("\n"))

  assert.throws(() => formula("0.2.0", missing), /no chauffeur-0\.2\.0-darwin-arm64/)
  assert.throws(() => parseSums("not a checksum line"), /not a SHA256SUMS line/)
})

test("the plugin shim sets opt defaults before importing, preserves overrides and follows an upgrade", () => {
  const root = mkdtempSync(join(tmpdir(), "chauffeur-shim-"))
  try {
    const opt = join(root, "opt/chauffeur")
    const link = join(root, "config/plugins/chauffeur.js")
    const ruby = formula("0.2.0", parseSums(fixtureSums("0.2.0")))
    const shim = ruby.match(/write <<~JS\n([\s\S]*?)\n    JS/)[1]
      .split("\n").map((line) => line.slice(6)).join("\n")
      .replaceAll("#{opt_pkgshare}", `${opt}/share/chauffeur`)
      .replaceAll("#{opt_bin}", `${opt}/bin`)
    const keg = (release) => {
      const path = join(root, `Cellar/chauffeur/${release}`)
      mkdirSync(join(path, "share/chauffeur/dist"), { recursive: true })
      writeFileSync(join(path, "share/chauffeur/dist/index.mjs"), `
        export const ChauffeurRpc = "rpc"
        export default { version: "${release}", bin: process.env.CHAUFFEUR_BIN, skills: process.env.CHAUFFEUR_SKILLS_DIR }
      `)
      writeFileSync(join(path, "share/chauffeur/opencode-plugin.js"), shim)
      return path
    }
    mkdirSync(join(root, "opt"))
    mkdirSync(join(root, "config/plugins"), { recursive: true })
    const first = keg("0.2.0")
    symlinkSync(first, opt)
    symlinkSync(`${opt}/share/chauffeur/opencode-plugin.js`, link)
    const env = { ...process.env }
    delete env.CHAUFFEUR_BIN
    delete env.CHAUFFEUR_SKILLS_DIR
    const load = (overrides = {}) => JSON.parse(execFileSync(process.execPath, ["--input-type=module", "-e", `
      const plugin = await import(${JSON.stringify(pathToFileURL(link).href)})
      console.log(JSON.stringify({ ...plugin.default, rpc: plugin.ChauffeurRpc }))
    `], { env: { ...env, ...overrides }, encoding: "utf8" }))
    assert.deepEqual(load(), { version: "0.2.0", bin: `${opt}/bin/chauffeur`, skills: `${opt}/share/chauffeur/skills`, rpc: "rpc" })
    assert.equal(load({ CHAUFFEUR_BIN: "/custom/chauffeur" }).bin, "/custom/chauffeur")
    const next = keg("0.2.1")
    unlinkSync(opt)
    symlinkSync(next, opt)
    rmSync(first, { recursive: true })
    assert.equal(load().version, "0.2.1")
  } finally {
    rmSync(root, { recursive: true })
  }
})
