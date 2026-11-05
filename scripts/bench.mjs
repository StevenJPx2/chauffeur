#!/usr/bin/env node
// Benchmarks this tree, or compares it with an earlier commit:
//
//   node scripts/bench.mjs                  # this tree, into target/bench/tree.txt
//   node scripts/bench.mjs v2~3             # v2~3 and this tree, then benchstat
//   node scripts/bench.mjs v2~3 -- -bench Redact -count 10
//
// Each crate's benches/ binary prints Go's benchmark format. The base commit
// is checked out in a temporary worktree and built in target/bench-base/, so
// its build is kept between runs. Go runs benchstat, pinned below.

import { spawnSync } from "node:child_process"
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)))
const BENCHSTAT =
  "golang.org/x/perf/cmd/benchstat@v0.0.0-20261009192801-be2c69fb417e"
const DEFAULTS = ["-benchtime", "1s", "-count", "6", "-benchmem"]

/** The base commit, if any, and the arguments for the benchmark binaries. */
export function parse(args) {
  const split = args.indexOf("--")
  const own = split === -1 ? args : args.slice(0, split)
  const passed = split === -1 ? [] : args.slice(split + 1)

  if (own.length > 1)
    throw new Error(`expected at most one base commit, got ${own.join(" ")}`)

  return { base: own[0], bench: [...DEFAULTS, ...passed] }
}

/** Runs every benchmark binary in `dir` and writes their output to `out`. */
function bench(dir, out, args, env = {}) {
  const cargo = ["bench", "--workspace", "--bench", "*", "--locked", "--", ...args]
  const ran = spawnSync("cargo", cargo, {
    cwd: dir,
    env: { ...process.env, ...env },
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"],
    maxBuffer: 64 * 1024 * 1024
  })

  if (ran.status !== 0) throw new Error(`cargo bench failed in ${dir}`)

  writeFileSync(out, ran.stdout)
}

/** Checks `rev` out in a temporary worktree, benchmarks it, and removes it. */
function benchAt(rev, out, args) {
  const dir = mkdtempSync(join(tmpdir(), "chauffeur-bench-"))
  const git = (...more) => spawnSync("git", more, { cwd: ROOT, stdio: "inherit" })

  if (git("worktree", "add", "--detach", dir, rev).status !== 0) {
    rmSync(dir, { recursive: true, force: true })
    throw new Error(`cannot check out ${rev}`)
  }

  try {
    bench(dir, out, args, { CARGO_TARGET_DIR: join(ROOT, "target/bench-base") })
  } finally {
    git("worktree", "remove", "--force", dir)
  }
}

function main() {
  const { base, bench: args } = parse(process.argv.slice(2))
  const folder = join(ROOT, "target/bench")
  const tree = join(folder, "tree.txt")

  mkdirSync(folder, { recursive: true })

  const baseFile = join(folder, "base.txt")

  if (base) benchAt(base, baseFile, args)
  bench(ROOT, tree, args)

  const columns = base ? [`base=${baseFile}`, `tree=${tree}`] : [`tree=${tree}`]
  const stat = spawnSync("go", ["run", BENCHSTAT, ...columns], { stdio: "inherit" })

  if (stat.status !== 0) throw new Error("benchstat failed; is Go installed?")
}

if (process.argv[1] === fileURLToPath(import.meta.url)) main()
