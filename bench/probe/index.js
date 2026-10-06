// chauffeur-bench probe: an OpenCode plugin that records what each agent
// request is made of, as sent to the provider, after every other plugin.
// Loaded from the run's plugin list; appends one JSON line per request to
// $BENCH_PROBE. Plain JavaScript with no imports beyond Node, so it loads
// from any folder.
import { appendFileSync, existsSync, writeFileSync } from "node:fs"

const OUT = process.env.BENCH_PROBE

const chars = (value) => (typeof value === "string" ? value.length : JSON.stringify(value ?? null).length)

// The first line of a system part names it, such as "# Code Mode".
const title = (text) => String(text).trimStart().split("\n", 1)[0].slice(0, 80)

const textOf = (part) => (typeof part === "string" ? part : part?.text ?? part?.content ?? "")

// One system text holds several instruction blocks; a "# " or "## " heading or a
// top-level tag such as <mcp_instructions> starts one.
const byHeading = (text) =>
  String(text)
    .split(/\n(?=#{1,2} |<[a-z_]+>)/)
    .map((block) => ({ part: "system", name: title(block), chars: block.length + 1 }))

function systemSections(body) {
  if (typeof body.system === "string") return byHeading(body.system)
  if (Array.isArray(body.system)) return body.system.flatMap((part) => byHeading(textOf(part)))
  // OpenAI Responses: one instructions string.
  if (typeof body.instructions === "string") return byHeading(body.instructions)

  return []
}

// The Anthropic auth plugin sends tools as "mcp_T" plus the base64 of the name.
function toolName(name) {
  const encoded = /^mcp_T([A-Za-z0-9+/_-]+=*)$/.exec(name ?? "")
  const decoded = encoded ? Buffer.from(encoded[1], "base64").toString("utf8") : ""

  return /^[\w.-]+$/.test(decoded) ? decoded : name ?? "?"
}

function messageSections(messages) {
  if (!Array.isArray(messages)) return []

  // Chat-style system or developer messages are system text; the rest is history.
  const system = messages.filter((message) => message?.role === "system" || message?.role === "developer")
  const history = messages.filter((message) => !system.includes(message))

  return [
    ...system.map((message) => ({ part: "system", name: title(textOf(Array.isArray(message.content) ? message.content[0] : message.content)), chars: chars(message) })),
    { part: "messages", name: `${history.length} messages`, chars: chars(history) },
  ]
}

const KNOWN = new Set(["system", "instructions", "tools", "messages", "input"])

function sections(body) {
  const tools = Array.isArray(body.tools)
    ? body.tools.map((tool) => ({ part: "tools", name: toolName(tool?.name ?? tool?.function?.name), chars: chars(tool) }))
    : []
  const other = Object.entries(body).filter(([key]) => !KNOWN.has(key))

  return [
    ...systemSections(body),
    ...tools,
    ...messageSections(body.messages ?? body.input),
    { part: "other", name: other.map(([key]) => key).join(","), chars: other.reduce((sum, [, value]) => sum + chars(value), 0) },
  ]
}

function record(transport, kind, text) {
  if (!OUT || kind !== "primary") return

  let body

  try {
    body = JSON.parse(text)
  } catch {
    body = undefined
  }

  const parsed = body && typeof body === "object"
  const line = {
    at: Date.now(),
    transport,
    total: text.length,
    namespaces: parsed ? namespaceCount(body) : 0,
    sections: parsed ? sections(body) : [],
  }

  // BENCH_PROBE_BODY=1 also keeps the first request as sent, for debugging.
  if (process.env.BENCH_PROBE_BODY === "1" && !existsSync(`${OUT}.first.json`)) writeFileSync(`${OUT}.first.json`, text)

  appendFileSync(OUT, `${JSON.stringify(line)}\n`)
}

const SETTLE_MS = 30_000

const STABLE_MS = 1_000

const POLL_MS = 250

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

// A fresh server connects MCP servers in the background, so a first request
// can go out before slow ones join the Code Mode catalog. Wait until none is
// pending and the tool list has not changed for a second, so every variant
// measures the catalog a long-running service would send.
async function mcpSettled(ctx) {
  const deadline = Date.now() + SETTLE_MS
  let count = -1
  let since = Date.now()

  while (Date.now() < deadline) {
    const listed = await ctx.mcp.list()
    const servers = Array.isArray(listed) ? listed : listed?.data ?? []
    const pending = servers.some((server) => (server?.status?.status ?? server?.status) === "pending")
    const tools = (await ctx.tool.list()).length

    if (tools !== count || pending) {
      count = tools
      since = Date.now()
    } else if (Date.now() - since >= STABLE_MS) {
      return
    }

    await sleep(POLL_MS)
  }
}

// Code Mode catalog lines name each namespace as "- name (".
const namespaceCount = (body) => {
  const text = [body.instructions, ...(Array.isArray(body.system) ? body.system.map(textOf) : [body.system])].join("\n")

  return (text.match(/^- [\w-]+ \(/gm) ?? []).length
}

export default {
  id: "chauffeur-bench-probe",
  async setup(ctx) {
    let settled

    await ctx.session.hook("prompt", () => (settled ??= mcpSettled(ctx).catch(() => undefined)))
    await ctx.session.hook("http.request", async (event) => {
      record("http", event.kind, await event.request.clone().text())
    })
    await ctx.session.hook("experimental.ws.send", (event) => {
      // A socket also carries control frames; only a new response is a request.
      if (!event.frame.includes('"response.create"')) return

      record("ws", event.kind, event.frame)
    })
  },
}
