// chauffeur-bench probe: an OpenCode plugin that records what each agent
// request is made of, as sent to the provider, after every other plugin.
// Loaded from the run's plugin list; appends one JSON line per request to
// $BENCH_PROBE. Plain JavaScript with no imports beyond Node, so it loads
// from any folder.
import { appendFileSync, existsSync, writeFileSync } from "node:fs"

const OUT = process.env.BENCH_PROBE

// BENCH_PROBE_BODY=1 also keeps the first request as sent, for debugging.
const KEEP_BODY = process.env.BENCH_PROBE_BODY === "1"

const SETTLE_MS = 30_000

const STABLE_MS = 1_000

const POLL_MS = 250

const KNOWN = new Set(["system", "instructions", "tools", "messages", "input"])

// The Anthropic auth plugin sends tools as "mcp_T" plus the base64 of the name.
const ENCODED = /^mcp_T([A-Za-z0-9+/_-]+=*)$/

const IDENTIFIER = /^[\w.-]+$/

// A Code Mode catalog line names a namespace as "- name (".
const NAMESPACE = /^- [\w-]+ \(/gm

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

const chars = (value) => (typeof value === "string" ? value.length : JSON.stringify(value ?? null).length)

// The first line of a block names it, such as "# Code Mode".
const title = (text) => String(text).trimStart().split("\n", 1)[0].slice(0, 80)

function textOf(part) {
  if (typeof part === "string") return part

  const { text, content } = Object(part)

  return String(text || content || "")
}

// One system text holds several instruction blocks; a "# " or "## " heading or a
// top-level tag such as <mcp_instructions> starts one.
const byHeading = (text) =>
  String(text)
    .split(/\n(?=#{1,2} |<[a-z_]+>)/)
    .map((block) => ({ part: "system", name: title(block), chars: block.length + 1 }))

// Anthropic's `system` (text or parts), or OpenAI Responses' `instructions`.
function systemTexts(body) {
  const system = body.system ?? body.instructions ?? []

  return (Array.isArray(system) ? system : [system]).map(textOf)
}

function toolName(name = "?") {
  const decoded = Buffer.from(ENCODED.exec(name)?.[1] ?? "", "base64").toString("utf8")

  return IDENTIFIER.test(decoded) ? decoded : name
}

function toolOf(tool) {
  const { name, function: declared } = Object(tool)

  return toolName(name ?? Object(declared).name)
}

const isSystemMessage = (message) => ["system", "developer"].includes(Object(message).role)

function contentText(message) {
  const { content } = Object(message)

  return textOf(Array.isArray(content) ? content[0] : content)
}

// Chat-style system or developer messages are system text; the rest is history.
function messageSections(messages) {
  if (!Array.isArray(messages)) return []

  const history = messages.filter((message) => !isSystemMessage(message))

  return [
    ...messages.filter(isSystemMessage).flatMap((message) => byHeading(contentText(message))),
    { part: "messages", name: `${history.length} messages`, chars: chars(history) },
  ]
}

function sections(body) {
  const tools = Array.isArray(body.tools) ? body.tools : []
  const other = Object.entries(body).filter(([key]) => !KNOWN.has(key))

  return [
    ...systemTexts(body).flatMap(byHeading),
    ...tools.map((tool) => ({ part: "tools", name: toolOf(tool), chars: chars(tool) })),
    ...messageSections(body.messages ?? body.input),
    { part: "other", name: other.map(([key]) => key).join(","), chars: other.reduce((sum, [, value]) => sum + chars(value), 0) },
  ]
}

const namespaceCount = (body) => (systemTexts(body).join("\n").match(NAMESPACE) ?? []).length

function parse(text) {
  try {
    return Object(JSON.parse(text))
  } catch {
    return {}
  }
}

function keepFirst(text) {
  const path = `${OUT}.first.json`

  if (KEEP_BODY && !existsSync(path)) writeFileSync(path, text)
}

function record(transport, kind, text) {
  if (!OUT || kind !== "primary") return

  const body = parse(text)
  const line = { at: Date.now(), transport, total: text.length, namespaces: namespaceCount(body), sections: sections(body) }

  keepFirst(text)
  appendFileSync(OUT, `${JSON.stringify(line)}\n`)
}

const isPending = (server) => [Object(server).status, Object(Object(server).status).status].includes("pending")

async function snapshot(ctx) {
  const listed = await ctx.mcp.list()
  const servers = Array.isArray(listed) ? listed : Object(listed).data ?? []

  return { pending: servers.some(isPending), tools: (await ctx.tool.list()).length }
}

// True once no server is pending and the tool count held for STABLE_MS.
function stability() {
  let tools = -1
  let since = Date.now()

  return (now) => {
    if (now.pending || now.tools !== tools) {
      tools = now.tools
      since = Date.now()
    }

    return Date.now() - since >= STABLE_MS
  }
}

// A fresh server connects MCP servers in the background, so a first request
// can go out before slow ones join the Code Mode catalog. Wait until they
// settle, so every variant measures the catalog a long-running service sends.
async function mcpSettled(ctx) {
  const deadline = Date.now() + SETTLE_MS
  const stable = stability()

  while (Date.now() < deadline && !stable(await snapshot(ctx))) await sleep(POLL_MS)
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
      if (event.frame.includes('"response.create"')) record("ws", event.kind, event.frame)
    })
  },
}
