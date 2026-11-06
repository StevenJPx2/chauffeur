#!/usr/bin/env node
// Unpack a release tarball, install it the two ways it ships, and run each
// for real: version, then a daemon that loads the shipped rule and nudges a
// nonconventional commit through `emit` and through the shipped OpenCode
// adapter, which `audit` then shows. No key and no network: a placeholder in
// the user's models.json stands in for Jev.
//
//   node scripts/smoke.mjs dist/chauffeur-<version>-<target>.tar.gz

import assert from "node:assert/strict"
import { execFileSync, spawn } from "node:child_process"
import { cpSync, existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { setTimeout } from "node:timers/promises"
import { pathToFileURL } from "node:url"
import { version } from "./package.mjs"

const STAND_IN = {
  models: { "stand-in": { format: "placeholder", otherwise: { yes: 0.94, no: 0.06 } } },
  tiers: { "system-one": ["stand-in"] },
}

const COMMIT = {
  event: "tool.after",
  session: "smoke",
  tool: "shell",
  input: { command: "git commit -m 'fixed stuff'" },
}

/** Lay the unpacked tarball out as Homebrew does for `name`: bin/<name>, share/<name>/. */
function install(unpacked, prefix, name) {
  mkdirSync(join(prefix, "bin"), { recursive: true })
  cpSync(join(unpacked, "bin/chauffeur"), join(prefix, "bin", name))
  cpSync(join(unpacked, "share/chauffeur"), join(prefix, "share", name), { recursive: true })

  return join(prefix, "bin", name)
}

async function waitFor(path, daemon, log) {
  for (let attempt = 0; attempt < 100; attempt++) {
    if (existsSync(path)) return
    if (daemon.exitCode !== null) throw new Error(`the daemon exited: ${log()}`)
    await setTimeout(50)
  }
  throw new Error(`no socket at ${path} after 5 s: ${log()}`)
}

/**
 * Loads the installed OpenCode adapter as OpenCode does, with a stand-in for
 * OpenCode's tool hooks and sessions, and runs its execute.after hook on a
 * commit. Returns the commit's result and the messages the adapter sent.
 */
async function throughAdapter(prefix, name, socket) {
  const server = join(prefix, "share", name, "opencode/server.js")
  const { default: adapter } = await import(pathToFileURL(server).href)
  const sent = []
  let hook

  await adapter.setup({
    options: { socket },
    session: { synthetic: async (input) => { sent.push(input) } },
    tool: { hook: async (_, callback) => { hook = callback } }
  })
  assert.equal(adapter.id, name)

  const call = {
    tool: "shell",
    sessionID: "smoke-opencode",
    input: COMMIT.input,
    status: "completed",
    result: { content: "[main 3f2c1a0] fixed stuff" }
  }

  await hook(call)

  return { content: call.result.content, sent }
}

async function run(binary, name, root, expected) {
  assert.equal(execFileSync(binary, ["version"], { encoding: "utf8" }).trim(), `${name} ${expected}`)

  const config = join(root, "config")
  // Short: a Unix socket's path is limited to about 100 bytes.
  const socket = join(root, "run", "s")
  mkdirSync(join(config, name), { recursive: true })
  writeFileSync(join(config, name, "models.json"), JSON.stringify(STAND_IN))

  const env = {
    ...process.env,
    XDG_CONFIG_HOME: config,
    XDG_STATE_HOME: join(root, "state"),
    CHAUFFEUR_SOCKET: socket
  }
  // The installed binary must find its shipped content by itself.
  delete env.CHAUFFEUR_DISTRIBUTION
  delete env.TYPESAFE_API_KEY

  let output = ""
  const daemon = spawn(binary, ["daemon", "run"], { env, stdio: ["ignore", "pipe", "pipe"] })
  const exited = new Promise((done) => daemon.once("exit", done))
  for (const stream of [daemon.stdout, daemon.stderr]) stream.on("data", (data) => { output = (output + data).slice(-8192) })

  try {
    await waitFor(socket, daemon, () => output)
    const call = (args, input = "") => JSON.parse(execFileSync(binary, args, { env, input, encoding: "utf8" }))
    const status = call(["daemon", "status"])
    assert.ok(status.rules.includes("conventional-commits"), JSON.stringify(status))
    const emitted = call(["emit"], JSON.stringify(COMMIT))
    assert.equal(emitted.actions[0]?.rule, "conventional-commits", JSON.stringify(emitted))
    assert.equal(emitted.actions[0]?.action, "message.append", JSON.stringify(emitted))
    const { content, sent } = await throughAdapter(join(root, "prefix"), name, socket)
    assert.equal(content, "[main 3f2c1a0] fixed stuff")
    assert.equal(sent.length, 1, JSON.stringify(sent))
    assert.equal(sent[0].delivery, "steer", JSON.stringify(sent))
    assert.match(sent[0].description, /^Chauffeur · conventional-commits: /, sent[0].description)
    assert.match(sent[0].text, /Conventional Commits/, sent[0].text)
    const audit = execFileSync(binary, ["audit"], { env, encoding: "utf8" }).trim().split("\n")
    assert.equal(audit.length, 2, audit.join("\n"))
    assert.equal(JSON.parse(audit[0]).returned, "message.append", audit[0])
  } finally {
    daemon.kill("SIGTERM")
    await Promise.race([exited, setTimeout(5000)])
    if (daemon.exitCode === null && daemon.signalCode === null) daemon.kill("SIGKILL")
  }
}

const archive = process.argv[2]
if (!archive) throw new Error("usage: smoke.mjs ARCHIVE")

const root = mkdtempSync(join(tmpdir(), "chauffeur-smoke-"))
try {
  const unpacked = join(root, "unpacked")
  mkdirSync(unpacked)
  execFileSync("tar", ["-xzf", resolve(archive), "-C", unpacked])
  const expected = version()

  for (const name of ["chauffeur", "chauffeur-next"]) {
    const home = join(root, name)
    await run(install(unpacked, join(home, "prefix"), name), name, home, expected)
  }
  console.log(`${archive}: version, shipped rule, nudge, OpenCode adapter and audit passed as chauffeur and chauffeur-next`)
} finally {
  rmSync(root, { recursive: true, force: true })
}
