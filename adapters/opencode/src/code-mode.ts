import type { HostTool } from "./host.js"
import { catalogEntry, TEXT_CODE_POINTS, type CodeModeNamespace } from "./protocol.js"
import { clip } from "./text.js"

const MAX_NAMESPACES = 64

const MAX_MATCHES = 8

/**
 * The host's Code Mode namespaces: tools the model reaches through `execute`
 * rather than the request's tool record. Each carries its size and its best
 * matches for the request, found the way Code Mode's own search would.
 */
export function codeModeNamespaces(tools: ReadonlyArray<HostTool>, request: string): CodeModeNamespace[] {
  return [...namespaceMembers(tools)].map(([name, members]) => ({
    name: clip(name, TEXT_CODE_POINTS),
    size: members.length,
    tools: rank(members, request).slice(0, MAX_MATCHES).map((tool) => catalogEntry(tool.id, tool.description, "")),
  }))
}

/**
 * Code Mode tools by namespace, at most `MAX_NAMESPACES`. OpenCode sends a
 * tool directly only when it opts out with `codemode: false`.
 */
export function namespaceMembers(tools: ReadonlyArray<HostTool>): Map<string, HostTool[]> {
  const byNamespace = new Map<string, HostTool[]>()

  for (const tool of tools) {
    if (tool.options?.codemode === false) continue

    const name = namespace(tool)

    if (!byNamespace.has(name) && byNamespace.size >= MAX_NAMESPACES) continue

    byNamespace.set(name, [...(byNamespace.get(name) ?? []), tool])
  }

  return byNamespace
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
