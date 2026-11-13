import type { Plugin } from "@opencode/plugin/effect"
import { Context, type Effect, Schema } from "effect"

/** The OpenCode plugin context: every seam a capability senses or acts at. */
export class Host extends Context.Service<Host, Plugin.Context>()("chauffeur/Host") {}

export type SessionID = Parameters<Plugin.Context["session"]["get"]>[0]["sessionID"]

/** A session ID an event carries as a plain string, such as a form's. */
export const toSessionID: (value: string) => SessionID = Schema.decodeSync(Schema.String.pipe(Schema.brand("SessionID")))

export type SessionInfo = Effect.Success<ReturnType<Plugin.Context["session"]["get"]>>

export type HistoryMessage = Effect.Success<ReturnType<Plugin.Context["session"]["context"]>>[number]

/** Messages since the most recent compaction, which begins a new context. */
export function currentContext(messages: ReadonlyArray<HistoryMessage>): HistoryMessage[] {
  const lastCompaction = messages.findLastIndex((message) => message.type === "compaction")

  return messages.slice(lastCompaction + 1)
}

export type HostTool = Effect.Success<ReturnType<Plugin.Context["tool"]["list"]>>[number]
