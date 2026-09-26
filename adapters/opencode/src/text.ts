/**
 * Truncate to whole code points so the result never splits a surrogate pair.
 * N code points encode to at most 4N UTF-8 bytes, which keeps signal fields
 * within the engine's byte bounds.
 */
export function clip(value: string, codePoints: number): string {
  return Array.from(value).slice(0, codePoints).join("")
}

/** Truncate to whole code points within `bytes` of UTF-8. */
export function clipBytes(value: string, bytes: number): string {
  let used = 0
  let kept = ""

  for (const point of value) {
    used += Buffer.byteLength(point, "utf8")

    if (used > bytes) break

    kept += point
  }

  return kept
}

/**
 * sourcefed tags the monitor events it delivers into a session with
 * `metadata.sourcefed`. They reach Chauffeur as integration events, so they are
 * never the user's words.
 */
export function isIntegrationMessage(metadata: { readonly sourcefed?: unknown } | undefined): boolean {
  return metadata?.sourcefed !== undefined
}
