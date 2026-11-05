const LIMIT = 256

/**
 * Text added to a call made inside Code Mode's `execute`, kept for
 * `execute`'s own result, which is what the agent reads. Both share the call
 * ID, and the inner call finishes first. Holds the latest 256 calls.
 */
export function carrier(limit = LIMIT) {
  const texts = new Map<string, readonly string[]>()

  return {
    put(id: string, appended: readonly string[]) {
      if (appended.length === 0) return

      texts.set(id, [...(texts.get(id) ?? []), ...appended])

      for (const oldest of texts.keys()) {
        if (texts.size <= limit) break

        texts.delete(oldest)
      }
    },

    take(id: string): readonly string[] {
      const found = texts.get(id) ?? []

      texts.delete(id)

      return found
    }
  }
}
