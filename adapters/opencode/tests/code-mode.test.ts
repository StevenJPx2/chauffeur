import { expect, test } from "bun:test"
import { codeModeNamespaces } from "../src/code-mode.js"
import type { HostTool } from "../src/host.js"

type Tool = { readonly id: string; readonly description: string; readonly options?: { readonly codemode?: boolean; readonly namespace?: string } }

function tools(list: ReadonlyArray<Tool>): ReadonlyArray<HostTool> {
  // SAFETY: codeModeNamespaces reads only each tool's id, description, and options.
  return list as ReadonlyArray<HostTool>
}

const catalog = tools([
  { id: "read", description: "Read a file" },
  { id: "browser_open", description: "Open a browser tab", options: { namespace: "browser.tabs" } },
  { id: "browser_type", description: "Type text into the page", options: { namespace: "browser.tabs" } },
  { id: "jira_issue", description: "Read a Jira issue" },
  { id: "native_hidden", description: "Native tool", options: { codemode: false } },
])

test("nothing is known before a request shows which tools are sent directly", () => {
  expect(codeModeNamespaces(catalog, null, "open a tab")).toEqual([])
})

test("namespaces group tools outside the request and rank the request's matches first", () => {
  const namespaces = codeModeNamespaces(catalog, new Set(["read"]), "type into the page")

  expect(namespaces.map((namespace) => [namespace.name, namespace.size])).toEqual([["browser", 2], ["jira", 1]])
  expect(namespaces[0]?.tools.map((tool) => tool.id)).toEqual(["browser_type", "browser_open"])
})
