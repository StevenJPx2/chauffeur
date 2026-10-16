#!/usr/bin/env node
// Unpack a native release tarball and exercise it without real credentials.
import assert from "node:assert/strict"
import { execFileSync, spawn } from "node:child_process"
import { mkdtempSync, mkdirSync, readFileSync, rmSync } from "node:fs"
import { createServer } from "node:net"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { setTimeout } from "node:timers/promises"
import { version } from "./package.mjs"

const archive = process.argv[2]
if (!archive) throw new Error("usage: smoke.mjs ARCHIVE")
const root = mkdtempSync(join(tmpdir(), "chauffeur-smoke-"))
let daemon
let exited
let log = ""

try {
  execFileSync("tar", ["-xzf", resolve(archive), "-C", root])
  const binary = join(root, "bin/chauffeur")
  const skills = join(root, "share/chauffeur/skills")
  const expected = version()
  assert.equal(execFileSync(binary, ["--version"], { encoding: "utf8" }).trim(), `chauffeur ${expected}`)
  execFileSync(binary, ["skill", "validate", join(skills, "rulebooks/goal.json")])
  assert.ok(readFileSync(join(root, "share/chauffeur/dist/index.mjs")).length > 0)

  const socket = createServer()
  await new Promise((done, reject) => {
    socket.once("error", reject)
    socket.listen(0, "127.0.0.1", done)
  })
  const { port } = socket.address()
  await new Promise((done) => socket.close(done))
  mkdirSync(join(root, "config"))
  const env = {
      ...process.env,
      HOME: root,
      TYPESAFE_API_KEY: "test-never-used",
      CHAUFFEUR_JUDGE: "jev",
      CHAUFFEUR_JUDGE_FALLBACK: "none",
      CHAUFFEUR_CONFIG_DIR: join(root, "config"),
      CHAUFFEUR_STATE_DIR: join(root, "state"),
      CHAUFFEUR_SKILLS_DIR: skills,
      CHAUFFEUR_SOURCEFED: "off",
      CHAUFFEUR_EMBEDDINGS: "off",
      CHAUFFEUR_MANAGED: "brew",
  }
  delete env.CHAUFFEUR_DAEMON_TOKEN
  daemon = spawn(binary, ["daemon", "--port", String(port)], {
    env,
    stdio: ["ignore", "pipe", "pipe"],
  })
  exited = new Promise((done, reject) => {
    daemon.once("exit", done)
    daemon.once("error", reject)
  })
  for (const stream of [daemon.stdout, daemon.stderr]) {
    stream.on("data", (data) => { log = (log + data).slice(-8192) })
  }
  const rpc = async (method, params = {}) => {
    const response = await fetch(`http://127.0.0.1:${port}/rpc`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ id: 1, method, params }),
      signal: AbortSignal.timeout(2000),
    })
    assert.ok(response.ok, `${method}: HTTP ${response.status}`)
    const reply = await response.json()
    assert.equal(reply.error, undefined, JSON.stringify(reply))
    return reply.result
  }
  let health
  for (let attempt = 0; attempt < 100; attempt++) {
    if (daemon.exitCode !== null) throw new Error(`daemon exited: ${log}`)
    try { health = await rpc("health"); break } catch { await setTimeout(100) }
  }
  assert.deepEqual(health, { ok: true, version: expected, managed: "brew" }, log)
  assert.ok(await rpc("texts", { host: "opencode" }))
  assert.deepEqual(await rpc("shutdown"), { ok: true })
  const code = await Promise.race([exited, setTimeout(5000).then(() => "timed out")])
  assert.equal(code, 0, `shutdown: ${code}; ${log}`)
  console.log(`packed daemon ${expected}: version, skills, health, texts, shutdown passed`)
} finally {
  if (daemon && daemon.exitCode === null) {
    daemon.kill("SIGKILL")
    await exited.catch(() => {})
  }
  rmSync(root, { recursive: true, force: true })
}
