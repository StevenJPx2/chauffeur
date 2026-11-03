#!/usr/bin/env node
// Replays ask_chauffeur requests against the running daemon and scores what
// each one hands over. See README.md beside this file.
import { execFileSync } from "node:child_process"
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"

const here = dirname(fileURLToPath(import.meta.url))
const args = process.argv.slice(2)
const flag = (name, fallback) => {
  const at = args.indexOf(`--${name}`)

  return at === -1 ? fallback : args[at + 1]
}
const repeats = Number(flag("repeats", "1"))
const only = flag("cases", "")?.split(",").filter(Boolean) ?? []
const chauffeur = flag("chauffeur", "chauffeur")
const catalog = JSON.parse(readFileSync(join(here, "catalog.json"), "utf8"))
// Each case's Code Mode namespaces with the tools Code Mode's own search finds for its need, as the plugin sends them.
const codeMode = JSON.parse(readFileSync(join(here, "code-mode.json"), "utf8"))
const cases = JSON.parse(readFileSync(join(here, "cases.json"), "utf8")).filter((test) => only.length === 0 || only.includes(test.id))
const scratch = mkdtempSync(join(tmpdir(), "chauffeur-ask-"))
const run = Date.now().toString(36)

/** The plugin's own waits: a prompt is admitted within 6 s, ask_chauffeur answers within 8 s. */
const DEADLINE_MS = { user_message: 6_000, agent_request: 8_000 }

function send(signal) {
  const file = join(scratch, "signal.json")

  writeFileSync(file, JSON.stringify(signal))

  return JSON.parse(execFileSync(chauffeur, ["signal", "--file", file, "--deadline-ms", String(DEADLINE_MS[signal.kind.type])], { encoding: "utf8" }))
}

/** What the agent was handed: skills, revealed tools, and Code Mode namespaces. */
function granted(effects, agentID) {
  return effects.filter((effect) => effect.agent_id === agentID).flatMap((effect) => {
    if (effect.type === "tools") return effect.reveal.map((tool) => `tools:${tool}`)
    if (effect.type !== "context") return []

    const namespaces = effect.label.startsWith("Code Mode") ? [...(effect.text ?? "").matchAll(/^## (\S+) \(/gm)].map((match) => `ns:${match[1]}`) : []

    return [...effect.skills.map((skill) => `skill:${skill}`), ...namespaces]
  })
}

function score(test, items) {
  const allowed = new Set([...test.want.flat(), ...test.allow])
  const missing = test.want.filter((group) => !group.some((item) => items.includes(item)))
  const extra = items.filter((item) => !allowed.has(item))
  const over = test.max !== undefined && items.length > test.max

  return { pass: missing.length === 0 && extra.length === 0 && !over, missing, extra, over }
}

/** What the agent was told came closest, when nothing was handed over. */
function closest(effects, agentID) {
  return effects.flatMap((effect) => (effect.type === "suggest" && effect.agent_id === agentID ? effect.names : []))
}

function report(result) {
  const notes = [
    result.missing.length ? `missing: ${result.missing.map((group) => group.join("|")).join("; ")}` : "",
    result.extra.length ? `extra: ${result.extra.join(", ")}` : "",
    result.over ? "too many" : "",
    result.closest.length ? `closest: ${result.closest.join(", ")}` : "",
  ].filter(Boolean)

  console.log(`${result.pass ? "pass" : "FAIL"}  ${result.id.padEnd(26)} ${String(result.ms).padStart(5)} ms  ${result.items.join(", ") || "(nothing)"}  ${notes.join("  ")}`)
}

const results = []
const rescore = flag("rescore", "")

// Grade a saved run again, against the current labels.
if (rescore) {
  const byID = new Map(cases.map((test) => [test.id, test]))

  for (const line of readFileSync(rescore, "utf8").split("\n").filter(Boolean)) {
    const saved = JSON.parse(line)
    const test = byID.get(saved.id)

    if (test) results.push({ ...saved, closest: closest(saved.effects, saved.effects[0]?.agent_id), ...score(test, saved.items) })
  }
}

for (let repeat = 0; repeat < (rescore ? 0 : repeats); repeat++) {
  for (const test of cases) {
    const agentID = `bench-ask-${run}-${repeat}-${test.id}`
    const attached = new Set(test.attached ?? [])
    const at = Math.floor(Date.now() / 1000)

    const namespaces = codeMode[test.id] ?? catalog.code_mode

    send({ agent_id: agentID, at, kind: { type: "user_message", text: "continue", first_in_context: true, skills: catalog.skills.filter((skill) => !attached.has(skill.id)), tools: [], code_mode: namespaces, workspace: "/tmp/ask-bench" } })

    const started = performance.now()
    const effects = send({ agent_id: agentID, at: at + 1, kind: { type: "agent_request", need: test.need, user_request: test.user_request, tools: [], code_mode: namespaces } })
    const ms = Math.round(performance.now() - started)
    const items = granted(effects, agentID)
    const result = { id: test.id, repeat, ms, items, closest: closest(effects, agentID), ...score(test, items), effects }

    results.push(result)
  }
}

for (const result of results) report(result)

const passed = results.filter((result) => result.pass).length
const handed = results.reduce((sum, result) => sum + result.items.length, 0) / results.length
const extras = results.reduce((sum, result) => sum + result.extra.length, 0)
const misses = results.filter((result) => result.missing.length > 0).length

console.log(`\n${passed}/${results.length} pass; ${misses} missed what was needed; ${extras} unneeded items; ${handed.toFixed(1)} items per request; median ${results.map((result) => result.ms).toSorted((a, b) => a - b)[Math.floor(results.length / 2)]} ms`)

const out = flag("out", "")

if (out) writeFileSync(out, results.map((result) => JSON.stringify(result)).join("\n") + "\n")

rmSync(scratch, { recursive: true, force: true })
