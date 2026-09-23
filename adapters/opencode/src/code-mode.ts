import type { Plugin } from "@opencode/plugin"
import type { SessionPrompt } from "@opencode/plugin/promise/session"
import type { CatalogEntry } from "./protocol.js"
import { clip } from "./text.js"

/** User messages record the Code Mode namespaces surfaced to them here. */
const SURFACED_METADATA_KEY = "chauffeur.surfaced"
const MAX_NAMESPACES = 64
const MAX_LISTED = 5
const DESCRIPTION_CODE_POINTS = 100
const TOOL_DESCRIPTION_CODE_POINTS = 160

type HostTool = Awaited<ReturnType<Plugin.Context["tool"]["list"]>>[number]
type HistoryMessage = Awaited<ReturnType<Plugin.Context["session"]["context"]>>[number]

/** Code Mode tools by namespace, and the catalog entries sent to the engine. */
export type CodeModeCatalog = {
  entries: CatalogEntry[]
  byNamespace: ReadonlyMap<string, HostTool[]>
}

/**
 * Tools the host does not put in the request's tool record reach the model
 * through Code Mode's `execute`, whose catalog shows each namespace only in
 * part. Namespaces already surfaced in this context are left out. Unknown
 * until the first request shows which tools are sent directly.
 */
export function codeModeCatalog(tools: ReadonlyArray<HostTool>, inRequests: ReadonlySet<string> | null, surfaced: ReadonlySet<string>): CodeModeCatalog {
  const byNamespace = new Map<string, HostTool[]>()

  if (inRequests === null) return { entries: [], byNamespace }

  for (const tool of tools) {
    const name = namespace(tool)

    // Native tools missing from requests were hidden or denied, not moved to Code Mode.
    if (inRequests.has(tool.id) || tool.options?.codemode === false || surfaced.has(name)) continue
    if (!byNamespace.has(name) && byNamespace.size >= MAX_NAMESPACES) continue

    byNamespace.set(name, [...(byNamespace.get(name) ?? []), tool])
  }

  const entries = [...byNamespace].map(([name, members]) => ({
    id: name,
    description: clip(`${members.length} tools, such as ${members.slice(0, 6).map((tool) => tool.id).join(", ")}`, DESCRIPTION_CODE_POINTS),
    bytes: 0,
  }))

  return { entries, byNamespace }
}

/** Namespaces already surfaced in this context. */
export function surfacedIn(context: ReadonlyArray<HistoryMessage>): Set<string> {
  return new Set(context.flatMap((message) => {
    const recorded = message.type === "user" ? message.metadata?.[SURFACED_METADATA_KEY] : undefined

    return Array.isArray(recorded) ? recorded.filter((name): name is string => typeof name === "string") : []
  }))
}

/**
 * Append a note naming each namespace's tools that best match the request,
 * as a text attachment on the prompt, so it reaches the first step without
 * touching the system prompt. Exact paths come from Code Mode's own `search`.
 */
export function surface(event: SessionPrompt, namespaces: string[], catalog: CodeModeCatalog): void {
  const sections = namespaces.flatMap((name) => {
    const tools = catalog.byNamespace.get(name)

    if (!tools) return []

    const lines = rank(tools, event.prompt.text)
      .slice(0, MAX_LISTED)
      .map((tool) => `- ${tool.id}: ${clip(tool.description, TOOL_DESCRIPTION_CODE_POINTS)}`)

    return [`## ${name} (${tools.length} tools)\n${lines.join("\n")}\nFind exact paths with \`search({ namespace: "${name}", query: "…" })\` inside \`execute\`.`]
  })

  if (sections.length === 0) return

  const note = `Chauffeur: Code Mode tools that fit this request. The catalog shows these namespaces only in part.\n\n${sections.join("\n\n")}\n`
  const previous = event.metadata?.[SURFACED_METADATA_KEY]

  event.prompt.files = [
    ...(event.prompt.files ?? []),
    { uri: `data:text/plain;base64,${Buffer.from(note, "utf8").toString("base64")}`, name: "chauffeur-code-mode-tools.txt" },
  ]
  event.metadata = { ...event.metadata, [SURFACED_METADATA_KEY]: [...(Array.isArray(previous) ? previous : []), ...namespaces] }
}

/** The tool's top-level Code Mode namespace, or its ID prefix. */
function namespace(tool: HostTool): string {
  const declared = tool.options?.namespace

  return (declared ? declared.split(".")[0] : tool.id.split("_")[0]) ?? tool.id
}

/** Tools sharing the most words with the request first, host order otherwise. */
function rank(tools: HostTool[], request: string): HostTool[] {
  const words = new Set(request.toLowerCase().match(/[a-z0-9]{3,}/g) ?? [])
  const score = (tool: HostTool): number =>
    [...new Set(`${tool.id} ${tool.description}`.toLowerCase().match(/[a-z0-9]{3,}/g) ?? [])].filter((word) => words.has(word)).length

  return tools.map((tool, index) => ({ tool, index, score: score(tool) }))
    .sort((a, b) => b.score - a.score || a.index - b.index)
    .map(({ tool }) => tool)
}
