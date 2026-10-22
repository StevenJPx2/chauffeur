/**
 * Truncate to whole code points so the result never splits a surrogate pair.
 * N code points encode to at most 4N UTF-8 bytes, which keeps signal fields
 * within the engine's byte bounds.
 */
export function clip(value: string, codePoints: number): string {
  return Array.from(value).slice(0, codePoints).join("")
}

/** The last `codePoints` code points, whole: the end of a text is nearest what came next. */
export function clipStart(value: string, codePoints: number): string {
  return Array.from(value).slice(-codePoints).join("")
}

/**
 * Both ends of a long text, whole code points, joined by an ellipsis: tools
 * report what happened first and what to do next last.
 */
export function clipEnds(value: string, codePoints: number): string {
  const points = Array.from(value)

  if (points.length <= codePoints) return value

  const head = Math.ceil((codePoints - 1) / 2)

  return [...points.slice(0, head), "…", ...points.slice(points.length - (codePoints - 1 - head))].join("")
}

/** The engine's bound on the user's own words, in UTF-8 bytes. */
const PROMPT_BYTES = 65_536

/**
 * The user's words as Chauffeur receives them: whole, never clipped. Text
 * over the engine's bound is left out instead, so the signal still reaches
 * the engine; the host still has the full prompt.
 */
export function userText(value: string): string {
  return Buffer.byteLength(value, "utf8") <= PROMPT_BYTES ? value : ""
}

/**
 * Both ends of a long text within `bytes` of UTF-8, whole code points, joined
 * by an ellipsis. The tail gets three quarters: a closing message ends on what
 * it needs from the user, often above a closing note such as its sources.
 */
export function clipEndsBytes(value: string, bytes: number): string {
  if (Buffer.byteLength(value, "utf8") <= bytes) return value

  const budget = bytes - Buffer.byteLength("…", "utf8")
  const head = clipBytes(value, Math.floor(budget / 4))
  const tail = [...clipBytes([...value].reverse().join(""), budget - Buffer.byteLength(head, "utf8"))].reverse().join("")

  return `${head}…${tail}`
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
