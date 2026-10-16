#!/usr/bin/env node
// Release artifacts, one tarball per target, and their checksums:
//
//   node scripts/package.mjs tarball --target darwin-arm64 --binary target/release/chauffeur --out dist
//   node scripts/package.mjs sums dist
//
// A tarball holds bin/chauffeur and share/chauffeur/{dist/index.mjs, skills,
// LICENSE}: the daemon, the plugin bundle beside its shipped skills, at the
// version Cargo.toml and the plugin's package.json both carry. Build the
// bundle first (`npm run build` in adapters/opencode).

import { execFileSync } from "node:child_process"
import { createHash } from "node:crypto"
import { cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { basename, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)))
const TARGETS = ["darwin-arm64", "darwin-x64", "linux-x64", "linux-arm64"]

/** The one version everything ships at; Cargo and the plugin must agree. */
export function version(root = ROOT) {
  const cargo = readFileSync(join(root, "Cargo.toml"), "utf8").match(/\[workspace\.package\][^[]*?\nversion = "([^"]+)"/)?.[1]
  const plugin = JSON.parse(readFileSync(join(root, "adapters/opencode/package.json"), "utf8")).version

  if (!cargo) throw new Error("Cargo.toml has no [workspace.package] version")
  if (cargo !== plugin) throw new Error(`Cargo.toml is ${cargo} but the plugin is ${plugin}`)

  return cargo
}

export function tarballName(release, target) {
  return `chauffeur-${release}-${target}.tar.gz`
}

/** Write `out/chauffeur-<version>-<target>.tar.gz` and return its path. */
export function tarball({ target, binary, out, root = ROOT }) {
  if (!TARGETS.includes(target)) throw new Error(`unknown target ${target}; expected one of ${TARGETS.join(", ")}`)

  const bundle = join(root, "adapters/opencode/dist/index.mjs")

  for (const path of [binary, bundle]) if (!existsSync(path)) throw new Error(`missing ${path}`)

  const release = version(root)
  const stage = mkdtempSync(join(tmpdir(), "chauffeur-package-"))
  const share = join(stage, "share/chauffeur")

  try {
    mkdirSync(join(stage, "bin"), { recursive: true })
    mkdirSync(join(share, "dist"), { recursive: true })
    cpSync(binary, join(stage, "bin/chauffeur"))
    cpSync(bundle, join(share, "dist/index.mjs"))
    cpSync(join(root, "skills"), join(share, "skills"), { recursive: true, filter: (path) => basename(path) !== ".DS_Store" })
    cpSync(join(root, "LICENSE"), join(share, "LICENSE"))

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

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [command, ...args] = process.argv.slice(2)

  if (command === "tarball") {
    const target = option(args, "--target")
    const binary = option(args, "--binary")
    const out = option(args, "--out") ?? "dist"

    if (!target || !binary) throw new Error("usage: package.mjs tarball --target TARGET --binary PATH [--out DIR]")

    console.log(tarball({ target, binary: resolve(binary), out }))
  } else if (command === "sums") {
    process.stdout.write(sums(args[0] ?? "dist"))
  } else {
    throw new Error("usage: package.mjs tarball --target TARGET --binary PATH [--out DIR] | sums [DIR]")
  }
}
