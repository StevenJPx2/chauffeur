import type { Result } from "@opencode/plugin/promise/tool"

export type { Result }

/** What the adapter reads of OpenCode's `execute.after` hook. */
export type ToolAfter = {
  readonly id: string
  readonly tool: string
  readonly sessionID: string
  readonly input: unknown
} & ({ readonly status: "completed"; result: Result } | { readonly status: "error" })

export const MAX_OUTPUT_BYTES = 64 * 1024

/** The `tool.after` event for a completed tool call, or none for a failed one. */
export function toolAfter(call: ToolAfter): Record<string, unknown> | undefined {
  if (call.status !== "completed") return undefined

  return {
    event: "tool.after",
    session: call.sessionID,
    tool: call.tool,
    input: call.input,
    output: clip(text(call.result), MAX_OUTPUT_BYTES)
  }
}

/** What the agent reads of a result: its text content, or its output. */
export function text(result: Result): string {
  const { content, output } = result

  if (typeof content === "string") return content

  if (content) {
    return content
      .flatMap((item) => (item.type === "text" ? [item.text] : []))
      .join("\n")
  }

  if (output === undefined) return ""

  return typeof output === "string" ? output : JSON.stringify(output)
}

export function clip(value: string, bytes: number): string {
  const encoded = new TextEncoder().encode(value)

  if (encoded.length <= bytes) return value

  return new TextDecoder().decode(encoded.subarray(0, bytes)).replace(/\uFFFD$/, "")
}
