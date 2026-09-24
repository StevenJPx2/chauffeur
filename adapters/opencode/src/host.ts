import type { Plugin } from "@opencode/plugin/effect"
import { Context, type Effect } from "effect"

/** The OpenCode plugin context: every seam a capability senses or acts at. */
export class Host extends Context.Service<Host, Plugin.Context>()("chauffeur/Host") {}

export type SessionID = Parameters<Plugin.Context["session"]["get"]>[0]["sessionID"]

export type HistoryMessage = Effect.Success<ReturnType<Plugin.Context["session"]["context"]>>[number]

export type HostTool = Effect.Success<ReturnType<Plugin.Context["tool"]["list"]>>[number]
