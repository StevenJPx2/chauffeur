import type { Plugin } from "@opencode/plugin/effect"
import { Effect } from "effect"
import { namespaceMembers } from "./code-mode.js"
import { Host, type HostTool, type SessionID } from "./host.js"
import type { HostEffect } from "./protocol.js"

type NamespacesEffect = Extract<HostEffect, { readonly type: "namespaces" }>

type Rules = NonNullable<Parameters<Plugin.Context["session"]["update"]>[0]["permissions"]>

type Rule = Rules[number]

/**
 * Withhold and restore Code Mode namespaces through the session's own
 * permission rules. OpenCode leaves a tool a rule wholly denies out of the
 * Code Mode catalog, its search, and MCP guidance, and session rules follow
 * the agent's, so a session deny takes effect from the next request. Other
 * rules on the session, such as the user's approvals, are kept as they are.
 */
export function applyNamespaces(sessionID: SessionID, effect: NamespacesEffect): Effect.Effect<void, unknown, Host> {
  return Effect.gen(function* () {
    const host = yield* Host
    const tools = yield* host.tool.list()
    const members = namespaceMembers(tools)
    const current: Rules = (yield* host.session.get({ sessionID })).permissions ?? []
    const revealed = effect.reveal.flatMap((name) => members.get(name) ?? [])
    const kept = current.filter((rule) => !(isWithholding(rule) && revealed.some((tool) => matches(rule.action, action(tool)))))

    const added = effect.hide
      .flatMap((name) => withholding(name, members.get(name) ?? [], tools))
      .filter((rule) => !kept.some((existing) => same(existing, rule)))

    const next = [...kept, ...added]

    if (next.length === current.length && next.every((rule, index) => same(rule, current[index]))) return

    yield* host.session.update({ sessionID, permissions: next })
  })
}

/**
 * The deny rules that withhold one namespace: one `<namespace>_*` rule when
 * that pattern covers exactly its members, else one rule per member.
 */
export function withholding(name: string, members: ReadonlyArray<HostTool>, all: ReadonlyArray<HostTool>): Rule[] {
  const pattern = `${name}_*`
  const covered = all.filter((tool) => matches(pattern, action(tool)))
  const exact = members.length > 0 && covered.length === members.length && members.every((tool) => matches(pattern, action(tool)))
  // Tools may share one permission action, such as `browser`'s.
  const actions = exact ? [pattern] : [...new Set(members.map(action))]

  return actions.map((denied) => ({ action: denied, resource: "*", effect: "deny" }))
}

/** The permission action OpenCode checks for a tool. */
function action(tool: HostTool): string {
  return tool.options?.permission ?? tool.id
}

const isWithholding = (rule: Rule): boolean => rule.effect === "deny" && rule.resource === "*"

const same = (left: Rule, right: Rule | undefined): boolean =>
  right !== undefined && left.action === right.action && left.resource === right.resource && left.effect === right.effect

/** OpenCode's wildcard: `*` matches any run of characters, `?` one. */
export function matches(pattern: string, value: string): boolean {
  const source = pattern.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*").replace(/\?/g, ".")

  return new RegExp(`^${source}$`, "s").test(value)
}
