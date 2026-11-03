// node --test scripts/*.test.mjs

import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { test } from "node:test"
import { formula, parseSums } from "./homebrew.mjs"
import { TARGETS, sums, tarball, version } from "./package.mjs"

/** A checkout with just what packaging reads, at `release`. */
function checkout(release = "1.0.0-rc.1") {
  const root = mkdtempSync(join(tmpdir(), "chauffeur-release-"))

  writeFileSync(join(root, "Cargo.toml"), `[workspace]\n\n[workspace.package]\nversion = "${release}"\nlicense = "MIT"\n`)
  for (const folder of ["models", "rules", "safety"]) mkdirSync(join(root, "distribution", folder), { recursive: true })
  writeFileSync(join(root, "distribution/models/jev.json"), "{}\n")
  writeFileSync(join(root, "distribution/rules/conventional-commits.json"), "{}\n")
  writeFileSync(join(root, "distribution/safety/redaction.json"), "{}\n")
  writeFileSync(join(root, "distribution/.DS_Store"), "")
  writeFileSync(join(root, "LICENSE"), "MIT\n")
  writeFileSync(join(root, "chauffeur"), "#!/bin/sh\n")

  return root
}

test("the version is the workspace's", () => {
  const root = checkout("1.0.0-rc.2")

  try {
    assert.equal(version(root), "1.0.0-rc.2")
    writeFileSync(join(root, "Cargo.toml"), "[workspace]\n")
    assert.throws(() => version(root), /no \[workspace\.package\] version/)
  } finally {
    rmSync(root, { recursive: true })
  }
})

test("a tarball holds the binary, the shipped content and the license, and SHA256SUMS lists it", () => {
  const root = checkout()
  const out = join(root, "out")

  try {
    const path = tarball({ target: "darwin-arm64", binary: join(root, "chauffeur"), out, root })
    const listed = execFileSync("tar", ["-tzf", path], { encoding: "utf8" }).split("\n")

    assert.ok(path.endsWith("chauffeur-1.0.0-rc.1-darwin-arm64.tar.gz"))
    for (const entry of [
      "bin/chauffeur",
      "share/chauffeur/models/jev.json",
      "share/chauffeur/rules/conventional-commits.json",
      "share/chauffeur/safety/redaction.json",
      "share/chauffeur/LICENSE",
    ]) {
      assert.ok(listed.includes(entry), `${entry} in ${listed.join(", ")}`)
    }
    assert.ok(!listed.some((entry) => entry.endsWith(".DS_Store")), listed.join(", "))
    assert.throws(() => tarball({ target: "win32-x64", binary: join(root, "chauffeur"), out, root }), /unknown target/)
    assert.throws(() => tarball({ target: "linux-x64", binary: join(root, "missing"), out, root }), /missing/)

    const text = sums(out)

    assert.match(text, /^[0-9a-f]{64} {2}chauffeur-1\.0\.0-rc\.1-darwin-arm64\.tar\.gz\n$/)
    assert.equal(readFileSync(join(out, "SHA256SUMS"), "utf8"), text)
  } finally {
    rmSync(root, { recursive: true })
  }
})

function fixtureSums(release) {
  return TARGETS.map((target, at) => `${String(at).repeat(64)}  chauffeur-${release}-${target}.tar.gz`).join("\n")
}

test("the formula names every target's tarball and checksum, and installs chauffeur-next", () => {
  const ruby = formula("1.0.0-rc.1", parseSums(fixtureSums("1.0.0-rc.1")))

  for (const [at, target] of TARGETS.entries()) {
    const url = `releases/download/v1.0.0-rc.1/chauffeur-1.0.0-rc.1-${target}.tar.gz"`
    assert.ok(ruby.includes(`${url}\n      sha256 "${String(at).repeat(64)}"`), target)
  }
  assert.match(ruby, /class ChauffeurNext < Formula/)
  assert.match(ruby, /on_macos do\n {4}on_arm do/)
  assert.match(ruby, /on_linux do\n {4}on_arm do/)
  assert.match(ruby, /bin\.install "bin\/chauffeur" => "chauffeur-next"/)
  assert.match(ruby, /pkgshare\.install Dir\["share\/chauffeur\/\*"\]/)
  assert.match(ruby, /run \[opt_bin\/"chauffeur-next", "daemon"\]/)
})

test("downloads can come from elsewhere, such as a local folder", () => {
  const ruby = formula("1.0.0-rc.1", parseSums(fixtureSums("1.0.0-rc.1")), "file:///tmp/dist")

  assert.match(ruby, /url "file:\/\/\/tmp\/dist\/chauffeur-1\.0\.0-rc\.1-linux-x64\.tar\.gz"/)
})

test("chauffeur-next needs a release candidate and every target", () => {
  assert.throws(() => formula("1.0.0", parseSums(fixtureSums("1.0.0"))), /needs a release candidate/)

  const missing = parseSums(fixtureSums("1.0.0-rc.1").split("\n").slice(1).join("\n"))

  assert.throws(() => formula("1.0.0-rc.1", missing), /no chauffeur-1\.0\.0-rc\.1-darwin-arm64/)
  assert.throws(() => parseSums("not a checksum line"), /not a SHA256SUMS line/)
})
