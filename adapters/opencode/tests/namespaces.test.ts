import { expect, test } from "bun:test"
import { Effect } from "effect"
import type { HostTool } from "../src/host.js"
import { applyNamespaces, matches, withholding } from "../src/namespaces.js"
import { fakeHost, install, noDaemon, sessionID } from "./support.js"

type Tool = { readonly id: string; readonly description: string; readonly options?: { readonly codemode?: boolean; readonly namespace?: string; readonly permission?: string } }

function tools(list: ReadonlyArray<Tool>): ReadonlyArray<HostTool> {
  // SAFETY: the namespace rules read only each tool's id and options.
  return list as ReadonlyArray<HostTool>
}

const catalog = tools([
  { id: "read", description: "", options: { codemode: false } },
  { id: "safari_open", description: "", options: { namespace: "safari" } },
  { id: "safari_close", description: "", options: { namespace: "safari" } },
  { id: "artifact_publish", description: "" },
  { id: "artifact_edit", description: "" },
  // A direct tool the artifact pattern would also catch.
  { id: "artifact_notes", description: "", options: { codemode: false } },
])

type Rule = { readonly action: string; readonly resource: string; readonly effect: "allow" | "deny" | "ask" }

const deny = (action: string): Rule => ({ action, resource: "*", effect: "deny" })

const session = sessionID("ses_ns")

test("a namespace is withheld by one pattern only when the pattern covers exactly its tools", () => {
  const safari = catalog.filter((tool) => tool.id.startsWith("safari_"))
  const artifact = catalog.filter((tool) => tool.id.startsWith("artifact_") && tool.options?.codemode !== false)

  expect(withholding("safari", safari, catalog)).toEqual([deny("safari_*")])
  expect(withholding("artifact", artifact, catalog)).toEqual([deny("artifact_publish"), deny("artifact_edit")])

  // OpenCode's browser tools share the `browser` permission action.
  const browser = tools([
    { id: "browser_open", description: "", options: { namespace: "browser", permission: "browser" } },
    { id: "browser_click", description: "", options: { namespace: "browser", permission: "browser" } },
  ])

  expect(withholding("browser", browser, browser)).toEqual([deny("browser")])
  expect(matches("safari_*", "safari_open")).toBe(true)
  expect(matches("safari_*", "safari")).toBe(false)
  expect(matches("a.b", "aXb")).toBe(false)
})

test("withholding keeps the user's rules, and restoring removes only Chauffeur's", async () => {
  const own: Rule = { action: "edit", resource: "/repo/**", effect: "allow" }
  let rules: ReadonlyArray<Rule> = [own]
  const updates: ReadonlyArray<Rule>[] = []

  const host = fakeHost({
    tool: { list: () => Effect.succeed(catalog) },
    session: {
      get: () => Effect.sync(() => ({ permissions: rules })),
      update: (input: { readonly permissions: ReadonlyArray<Rule> }) => Effect.sync(() => {
        rules = input.permissions
        updates.push(rules)
      }),
    },
  })

  const run = (hide: string[], reveal: string[]) =>
    install(applyNamespaces(session, { type: "namespaces", agent_id: String(session), hide, reveal }).pipe(Effect.orDie), host, noDaemon)
      .then((plugin) => plugin.close())

  await run(["safari", "artifact"], [])
  expect(rules).toEqual([own, deny("safari_*"), deny("artifact_publish"), deny("artifact_edit")])

  // Withholding again changes nothing.
  await run(["safari"], [])
  expect(updates).toHaveLength(1)

  await run([], ["safari"])
  expect(rules).toEqual([own, deny("artifact_publish"), deny("artifact_edit")])
})
