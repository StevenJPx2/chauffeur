#!/usr/bin/env node
// Where each request's skill lands in the ranking that decides which skills
// are judged: embeddings (the daemon's), BM25, and both fused. See README.md.
import { readFileSync } from "node:fs"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"

const here = dirname(fileURLToPath(import.meta.url))
const service = process.env.CHAUFFEUR_EMBEDDINGS ?? "http://127.0.0.1:18791"
const catalog = JSON.parse(readFileSync(join(here, "catalog.json"), "utf8")).skills
const set = process.argv.includes("--set") ? process.argv[process.argv.indexOf("--set") + 1] : "ranking.json"
const cases = JSON.parse(readFileSync(join(here, set), "utf8"))
/** The daemon judges this many ranked skills per prompt or request. */
const JUDGED = 16

// The text each skill is embedded and indexed as, as the daemon builds it.
const documents = catalog.map((skill) => `${skill.id}: ${skill.description}`)

async function embed(kind, texts) {
  const { model } = await (await fetch(`${service}/health`)).json()
  const vectors = []

  for (let at = 0; at < texts.length; at += 32) {
    const reply = await fetch(`${service}/embed`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ model, kind, texts: texts.slice(at, at + 32) }) })

    vectors.push(...(await reply.json()).vectors)
  }

  return vectors
}

const dot = (a, b) => a.reduce((sum, value, index) => sum + value * b[index], 0)
const norm = (a) => Math.sqrt(dot(a, a))
const cosine = (a, b) => dot(a, b) / (norm(a) * norm(b))

const STOP = new Set("a an and are as at be by for from how i in is it its me my of on or so that the this to use when with you your what which why will can do does any all".split(" "))
const words = (text) => (text.toLowerCase().match(/[a-z0-9]+/g) ?? []).filter((word) => !STOP.has(word))

/** A light suffix stemmer: "deployed", "deployment" and "deploys" all become "deploy". */
const SUFFIXES = ["ations", "ation", "ments", "ment", "ings", "ing", "ers", "er", "ies", "ied", "ed", "es", "s"]
const stem = (word) => {
  for (const suffix of SUFFIXES) {
    if (word.length - suffix.length >= 3 && word.endsWith(suffix)) return suffix === "ies" || suffix === "ied" ? `${word.slice(0, -3)}y` : word.slice(0, -suffix.length)
  }

  return word
}
const stemmed = (text) => words(text).map(stem)

/** As the daemon stems (crates/daemon/src/lexical.rs): also without a final "e" or a doubled final consonant. */
const trim = (root) => {
  const last = root.at(-1)
  const doubled = last === root.at(-2) && /[b-df-hj-kmnp-rt-z]/.test(last ?? "")

  return root.length > 3 && (last === "e" || doubled) ? root.slice(0, -1) : root
}
const daemonStemmed = (text) => words(text).map((word) => trim(stem(word)))

/** Okapi BM25 over the skills' ID and description; `nameWeight` repeats the ID's words. */
function bm25({ k1 = 1.2, b = 0.75, tokens = words, nameWeight = 1 } = {}) {
  const docs = catalog.map((skill) => [...Array.from({ length: nameWeight }, () => tokens(skill.id.replaceAll("-", " "))).flat(), ...tokens(skill.description)])
  const average = docs.reduce((sum, doc) => sum + doc.length, 0) / docs.length
  const frequency = new Map()

  for (const doc of docs) for (const word of new Set(doc)) frequency.set(word, (frequency.get(word) ?? 0) + 1)

  return (query) => docs.map((doc) => tokens(query).reduce((score, word) => {
    const n = frequency.get(word) ?? 0
    const tf = doc.filter((term) => term === word).length

    if (n === 0 || tf === 0) return score

    const idf = Math.log(1 + (docs.length - n + 0.5) / (n + 0.5))

    return score + idf * (tf * (k1 + 1)) / (tf + k1 * (1 - b + b * doc.length / average))
  }, 0))
}

/** Skill indices, best first. */
const order = (scores) => scores.map((score, index) => [score, index]).sort((a, b) => b[0] - a[0]).map(([, index]) => index)

/** Reciprocal rank fusion of two orders. */
function fuse(first, second, k = 60) {
  const scores = new Array(catalog.length).fill(0)

  for (const ranking of [first, second]) ranking.forEach((index, rank) => { scores[index] += 1 / (k + rank + 1) })

  return order(scores)
}

const skillVectors = await embed("document", documents)
const queryVectors = []
const embedTimes = []

// One request at a time, as the daemon embeds each prompt.
for (const test of cases) {
  const started = performance.now()

  queryVectors.push((await embed("query", [test.query]))[0])
  embedTimes.push(performance.now() - started)
}

const lexical = bm25()
const variants = { "bm25+stem": bm25({ tokens: stemmed }), "bm25 daemon": bm25({ tokens: daemonStemmed }) }
const bm25Times = cases.map((test) => {
  const started = performance.now()

  lexical(test.query)

  return performance.now() - started
})
const median = (times) => times.toSorted((a, b) => a - b)[Math.floor(times.length / 2)]
const p90 = (times) => times.toSorted((a, b) => a - b)[Math.floor(times.length * 0.9)]
const methods = { embeddings: [], bm25: [], "bm25+stem": [], "bm25 daemon": [], fused: [] }

for (const [index, test] of cases.entries()) {
  const semantic = order(skillVectors.map((vector) => cosine(queryVectors[index], vector)))
  const keyword = order(lexical(test.query))
  const orders = { embeddings: semantic, bm25: keyword, ...Object.fromEntries(Object.entries(variants).map(([name, score]) => [name, order(score(test.query))])), fused: fuse(semantic, keyword) }

  for (const [method, ranking] of Object.entries(orders)) {
    const target = catalog.findIndex((skill) => skill.id === test.want)

    methods[method].push({ ...test, rank: ranking.indexOf(target) + 1 })
  }
}

const summary = (results, filter = () => true) => {
  const chosen = results.filter(filter)
  const at = (k) => chosen.filter((result) => result.rank <= k).length

  return `top1 ${at(1)}/${chosen.length}  top5 ${at(5)}  top${JUDGED} ${at(JUDGED)}  top24 ${at(24)}  MRR ${(chosen.reduce((sum, result) => sum + 1 / result.rank, 0) / chosen.length).toFixed(2)}`
}

for (const [method, results] of Object.entries(methods)) {
  console.log(`${method.padEnd(11)} all: ${summary(results)} | named: ${summary(results, (result) => result.style === "named")} | paraphrased: ${summary(results, (result) => result.style === "paraphrase")}`)
}

console.log(`\nper request, ranking ${catalog.length} skills: embeddings median ${median(embedTimes).toFixed(1)} ms (p90 ${p90(embedTimes).toFixed(1)}), BM25 median ${median(bm25Times).toFixed(2)} ms (p90 ${p90(bm25Times).toFixed(2)})`)
console.log(`\nrank of the wanted skill (${Object.keys(methods).join(" / ")}):`)

for (const [index, test] of cases.entries()) {
  const ranks = Object.values(methods).map((results) => results[index].rank)

  if (ranks.some((rank) => rank > 5)) console.log(`  ${ranks.map((rank) => String(rank).padStart(3)).join(" /")}  ${test.want.padEnd(26)} ${test.query}`)
}
