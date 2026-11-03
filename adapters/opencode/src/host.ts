import type { Plugin } from "@opencode/plugin/effect"
import { Context, type Effect } from "effect"

/** The OpenCode plugin context: every seam a capability senses or acts at. */
export class Host extends Context.Service<Host, Plugin.Context>()("chauffeur/Host") {}

export type SessionID = Parameters<Plugin.Context["session"]["get"]>[0]["sessionID"]

export type SessionInfo = Effect.Success<ReturnType<Plugin.Context["session"]["get"]>>

export type HistoryMessage = Effect.Success<ReturnType<Plugin.Context["session"]["context"]>>[number]

/** Messages since the most recent compaction, which begins a new context. */
export function currentContext(messages: ReadonlyArray<HistoryMessage>): HistoryMessage[] {
  const lastCompaction = messages.findLastIndex((message) => message.type === "compaction")

  return messages.slice(lastCompaction + 1)
}

export type HostTool = Effect.Success<ReturnType<Plugin.Context["tool"]["list"]>>[number]
