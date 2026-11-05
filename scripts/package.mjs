#!/usr/bin/env node
// Release artifacts, one tarball per target, and their checksums:
//
//   node scripts/package.mjs version
//   node scripts/package.mjs tarball --target darwin-arm64 --binary target/release/chauffeur --out dist
//   node scripts/package.mjs sums dist
//
// A tarball holds bin/chauffeur and share/chauffeur/: the shipped models,
// rules and safety patterns from distribution/, the OpenCode adapter built in
// adapters/opencode/dist/, and the license. The binary
// finds share/<its name>/ beside its own folder, so the same tarball installs
// as chauffeur or, renamed, as chauffeur-next.

import { execFileSync } from "node:child_process"
import { createHash } from "node:crypto"
import { cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { basename, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)))
export const TARGETS = ["darwin-arm64", "darwin-x64", "linux-arm64", "linux-x64"]

/** The workspace's version, which every crate and the release share. */
export function version(root = ROOT) {
  const cargo = readFileSync(join(root, "Cargo.toml"), "utf8")
  const found = cargo.match(/\[workspace\.package\][^[]*?\nversion = "([^"]+)"/)?.[1]

  if (!found) throw new Error("Cargo.toml has no [workspace.package] version")

  return found
}

export function tarballName(release, target) {
  return `chauffeur-${release}-${target}.tar.gz`
}

/** Write `out/chauffeur-<version>-<target>.tar.gz` and return its path. */
export function tarball({ target, binary, out, root = ROOT }) {
  if (!TARGETS.includes(target)) throw new Error(`unknown target ${target}; expected one of ${TARGETS.join(", ")}`)
  if (!existsSync(binary)) throw new Error(`missing ${binary}`)

  const adapter = join(root, "adapters/opencode/dist/server.js")

  if (!existsSync(adapter)) {
    throw new Error(`missing ${adapter}; run bun run build in adapters/opencode`)
  }

  const release = version(root)
  const stage = mkdtempSync(join(tmpdir(), "chauffeur-package-"))
  const share = join(stage, "share/chauffeur")
  const skip = (path) => basename(path) !== ".DS_Store"

  try {
    mkdirSync(join(stage, "bin"), { recursive: true })
    cpSync(binary, join(stage, "bin/chauffeur"))
    cpSync(join(root, "distribution"), share, { recursive: true, filter: skip })
    cpSync(join(root, "LICENSE"), join(share, "LICENSE"))
    mkdirSync(join(share, "opencode"))
    cpSync(adapter, join(share, "opencode/server.js"))

    mkdirSync(out, { recursive: true })
    const path = resolve(out, tarballName(release, target))

    execFileSync("tar", ["-czf", path, "-C", stage, "bin", "share"])

    return path
  } finally {
    rmSync(stage, { recursive: true, force: true })
  }
}

/** Write `SHA256SUMS` for every tarball in `folder` and return its text. */
export function sums(folder) {
  const lines = readdirSync(folder)
    .filter((name) => name.endsWith(".tar.gz"))
    .toSorted()
    .map((name) => `${createHash("sha256").update(readFileSync(join(folder, name))).digest("hex")}  ${name}`)

  if (lines.length === 0) throw new Error(`no tarballs in ${folder}`)

  const text = `${lines.join("\n")}\n`

  writeFileSync(join(folder, "SHA256SUMS"), text)

  return text
}

function option(args, name) {
  const at = args.indexOf(name)

  return at === -1 ? undefined : args[at + 1]
}

const USAGE = "usage: package.mjs version | tarball --target TARGET --binary PATH [--out DIR] | sums [DIR]"

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [command, ...args] = process.argv.slice(2)

  if (command === "version") {
    console.log(version())
  } else if (command === "tarball") {
    const target = option(args, "--target")
    const binary = option(args, "--binary")

    if (!target || !binary) throw new Error(USAGE)

    console.log(tarball({ target, binary: resolve(binary), out: option(args, "--out") ?? "dist" }))
  } else if (command === "sums") {
    process.stdout.write(sums(args[0] ?? "dist"))
  } else {
    throw new Error(USAGE)
  }
}
