/**
 * Truncate to whole code points so the result never splits a surrogate pair.
 * N code points encode to at most 4N UTF-8 bytes, which keeps signal fields
 * within the engine's byte bounds.
 */
export function clip(value: string, codePoints: number): string {
  return Array.from(value).slice(0, codePoints).join("")
}

/**
 * sourcefed tags the monitor events it delivers into a session with
 * `metadata.sourcefed`. They reach Chauffeur as integration events, so they are
 * never the user's words.
 */
export function isIntegrationMessage(metadata: Record<string, unknown> | undefined): boolean {
  return metadata?.sourcefed !== undefined
}
