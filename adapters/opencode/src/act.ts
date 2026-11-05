import type { Action } from "./daemon.ts"
import type { Result } from "./event.ts"

type Content = Exclude<NonNullable<Result["content"]>, string>[number]

export type Applied = {
  readonly result: Result
  readonly appended: readonly string[]
  readonly skipped: readonly Action[]
}

/**
 * Carries out the actions on a completed tool call's result. `message.append`
 * with `delivery: "now"` adds its text parts; anything else is skipped.
 */
export function apply(result: Result, actions: readonly Action[]): Applied {
  const texts: string[] = []
  const skipped: Action[] = []

  for (const action of actions) {
    const found = appendNow(action)

    if (found) {
      texts.push(...found)
    } else {
      skipped.push(action)
    }
  }

  return { result: append(result, texts), appended: texts, skipped }
}

function appendNow(action: Action): string[] | undefined {
  if (action.action !== "message.append") return undefined

  const payload = action.payload as
    { parts?: readonly { text?: unknown }[]; delivery?: unknown } | undefined

  if (payload?.delivery !== "now" || !Array.isArray(payload.parts)) return undefined

  const texts = payload.parts.flatMap((part) =>
    typeof part.text === "string" ? [part.text] : []
  )

  return texts.length > 0 ? texts : undefined
}

/** Adds texts after a result's content, or after its output when it has none. */
export function append(result: Result, texts: readonly string[]): Result {
  const { content } = result

  if (texts.length === 0) return result

  if (Array.isArray(content)) {
    const items: Content[] = texts.map((text) => ({
      type: "text",
      text: `\n\n${text}`
    }))

    return { ...result, content: [...content, ...items] }
  }

  const before = typeof content === "string" ? content : textOfOutput(result)
  const joined = [before, ...texts].filter((part) => part !== "").join("\n\n")

  return { ...result, content: joined }
}

function textOfOutput({ output }: Result): string {
  if (output === undefined) return ""

  return typeof output === "string" ? output : JSON.stringify(output, null, 2)
}
