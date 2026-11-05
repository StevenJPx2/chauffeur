import { expect, test } from "bun:test"
import type { Plugin } from "@opencode/plugin"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { adapter } from "../src/index.ts"
import { NUDGE, standIn } from "./stand-in.ts"

type Hook = (call: Record<string, unknown>) => Promise<void>

/** Sets the adapter up in a stand-in OpenCode and returns its execute.after hook. */
async function load(
  options: Record<string, unknown> | undefined,
  log: (line: string) => void
) {
  const hooks = new Map<string, Hook>()
  const ctx = {
    options,
    tool: {
      hook: async (name: string, callback: Hook) => {
        hooks.set(name, callback)

        return { dispose: async () => {} }
      }
    }
  }

  await adapter(import.meta.url, log).setup(
    ctx as unknown as Parameters<Plugin.Plugin["setup"]>[0]
  )

  const hook = hooks.get("execute.after")

  if (!hook) throw new Error("the adapter hooked no execute.after")

  return hook
}

function commit(status: "completed" | "error") {
  return {
    id: "toolu_1",
    tool: "shell",
    sessionID: "ses_1",
    input: { command: "git commit -m 'fixed stuff'" },
    status,
    result: { content: "[v2 3f2c1a0] fixed stuff" }
  }
}

test("a nudge from the rules ends the commit's tool result", async () => {
  const daemon = await standIn(() => ({ result: { actions: [NUDGE] } }))
  const logged: string[] = []

  try {
    const hook = await load({ socket: daemon.socket, deadline_ms: 200 }, (l) =>
      logged.push(l)
    )
    const call = commit("completed")

    await hook(call)

    expect(call.result.content).toEndWith(
      "Use Conventional Commits, such as `feat(scope): …`."
    )
    expect(daemon.requests[0]?.params.deadline_ms).toBe(200)
    expect(logged).toEqual([])
  } finally {
    await daemon.close()
  }
})

test("a failed call is not sent", async () => {
  const daemon = await standIn(() => ({ result: { actions: [NUDGE] } }))

  try {
    const hook = await load({ socket: daemon.socket }, () => {})

    await hook(commit("error"))

    expect(daemon.requests).toEqual([])
  } finally {
    await daemon.close()
  }
})

test("with no daemon the result stays, and the log says so once a minute", async () => {
  const logged: string[] = []
  const socket = join(tmpdir(), "chauffeur-missing", "s")
  const hook = await load({ socket, deadline_ms: 50 }, (line) => logged.push(line))
  const first = commit("completed")
  const second = commit("completed")

  await hook(first)
  await hook(second)

  expect(first.result.content).toBe("[v2 3f2c1a0] fixed stuff")
  expect(second.result.content).toBe("[v2 3f2c1a0] fixed stuff")
  expect(logged).toHaveLength(1)
  expect(logged[0]).toContain(`nothing sent for shell`)
  expect(logged[0]).toContain(socket)
})

test("with no options the adapter still loads and reaches the daemon", async () => {
  const daemon = await standIn(() => ({ result: { actions: [NUDGE] } }))
  const previous = process.env.CHAUFFEUR_SOCKET

  process.env.CHAUFFEUR_SOCKET = daemon.socket

  try {
    const hook = await load(undefined, () => {})
    const call = commit("completed")

    await hook(call)

    expect(call.result.content).toEndWith("such as `feat(scope): …`.")
  } finally {
    if (previous === undefined) {
      delete process.env.CHAUFFEUR_SOCKET
    } else {
      process.env.CHAUFFEUR_SOCKET = previous
    }

    await daemon.close()
  }
})

test("a nudge for a call inside Code Mode also ends execute's result", async () => {
  const daemon = await standIn((request) => {
    const event = request.params.event as { tool: string }

    return { result: { actions: event.tool === "shell" ? [NUDGE] : [] } }
  })

  try {
    const hook = await load({ socket: daemon.socket }, () => {})
    const execute = {
      id: "toolu_1",
      tool: "execute",
      sessionID: "ses_1",
      input: { code: 'return await tools.shell({ command: "git commit -m wip" })' },
      status: "completed",
      result: { content: [{ type: "text", text: '{ "exit": 0 }' }] }
    }
    const other = { ...execute, id: "toolu_2" }

    await hook(commit("completed"))
    await hook(execute)
    await hook(other)

    expect(execute.result.content).toEqual([
      { type: "text", text: '{ "exit": 0 }' },
      { type: "text", text: `\n\n${NUDGE.payload.parts[0]!.text}` }
    ])

    expect(other.result.content).toHaveLength(1)
  } finally {
    await daemon.close()
  }
})

test("the plugin is named after its installation", () => {
  const shipped = "file:///opt/homebrew/share/chauffeur-next/opencode/server.js"

  expect(adapter(shipped).id).toBe("chauffeur-next")
  expect(adapter(import.meta.url).id).toBe("chauffeur")
})
