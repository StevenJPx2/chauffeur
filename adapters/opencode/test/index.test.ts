import { expect, test } from "bun:test"
import type { Plugin } from "@opencode/plugin"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { adapter } from "../src/index.ts"
import { NUDGE, standIn } from "./stand-in.ts"

type Hook = (call: Record<string, unknown>) => Promise<void>

/**
 * Sets the adapter up in a stand-in OpenCode. Returns its execute.after hook
 * and each synthetic message it sends; `refuse` makes sending fail.
 */
async function load(
  options: Record<string, unknown> | undefined,
  log: (line: string) => void,
  refuse = false
) {
  const hooks = new Map<string, Hook>()
  const sent: Record<string, unknown>[] = []
  const ctx = {
    options,
    session: {
      synthetic: async (input: Record<string, unknown>) => {
        if (refuse) throw new Error("session is gone")

        sent.push(input)
      }
    },
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

  return { hook, sent }
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

test("a nudge from the rules is its own message, steered into the turn", async () => {
  const daemon = await standIn(() => ({ result: { actions: [NUDGE] } }))
  const logged: string[] = []

  try {
    const { hook, sent } = await load(
      { socket: daemon.socket, deadline_ms: 200 },
      (line) => logged.push(line)
    )
    const call = commit("completed")

    await hook(call)

    expect(sent).toEqual([
      {
        sessionID: "ses_1",
        text: NUDGE.payload.parts[0]!.text,
        description: "Chauffeur · conventional-commits: commit message isn't conventional",
        metadata: { chauffeur: { rule: "conventional-commits" } },
        delivery: "steer"
      }
    ])
    expect(call.result.content).toBe("[v2 3f2c1a0] fixed stuff")
    expect(daemon.requests[0]?.params.deadline_ms).toBe(200)
    expect(logged).toEqual([])
  } finally {
    await daemon.close()
  }
})

test("a wake is queued with a wake-up", async () => {
  const wake = { ...NUDGE, payload: { ...NUDGE.payload, delivery: "wake" } }
  const daemon = await standIn(() => ({ result: { actions: [wake] } }))

  try {
    const { hook, sent } = await load({ socket: daemon.socket }, () => {})

    await hook(commit("completed"))

    expect(sent[0]).toMatchObject({ delivery: "queue", resume: true })
  } finally {
    await daemon.close()
  }
})

test("a failed call is not sent", async () => {
  const daemon = await standIn(() => ({ result: { actions: [NUDGE] } }))

  try {
    const { hook, sent } = await load({ socket: daemon.socket }, () => {})

    await hook(commit("error"))

    expect(daemon.requests).toEqual([])
    expect(sent).toEqual([])
  } finally {
    await daemon.close()
  }
})

test("with no daemon nothing is sent, and the log says so once a minute", async () => {
  const logged: string[] = []
  const socket = join(tmpdir(), "chauffeur-missing", "s")
  const { hook, sent } = await load({ socket, deadline_ms: 50 }, (line) =>
    logged.push(line)
  )
  const first = commit("completed")

  await hook(first)
  await hook(commit("completed"))

  expect(first.result.content).toBe("[v2 3f2c1a0] fixed stuff")
  expect(sent).toEqual([])
  expect(logged).toHaveLength(1)
  expect(logged[0]).toContain(`nothing sent for shell`)
  expect(logged[0]).toContain(socket)
})

test("a message OpenCode refuses is logged once a minute", async () => {
  const daemon = await standIn(() => ({ result: { actions: [NUDGE] } }))
  const logged: string[] = []

  try {
    const { hook } = await load(
      { socket: daemon.socket },
      (line) => logged.push(line),
      true
    )

    await hook(commit("completed"))
    await hook(commit("completed"))

    expect(logged).toEqual([
      "chauffeur: nothing delivered for conventional-commits: session is gone"
    ])
  } finally {
    await daemon.close()
  }
})

test("with no options the adapter still loads and reaches the daemon", async () => {
  const daemon = await standIn(() => ({ result: { actions: [NUDGE] } }))
  const previous = process.env.CHAUFFEUR_SOCKET

  process.env.CHAUFFEUR_SOCKET = daemon.socket

  try {
    const { hook, sent } = await load(undefined, () => {})

    await hook(commit("completed"))

    expect(sent).toHaveLength(1)
  } finally {
    if (previous === undefined) {
      delete process.env.CHAUFFEUR_SOCKET
    } else {
      process.env.CHAUFFEUR_SOCKET = previous
    }

    await daemon.close()
  }
})

test("a call inside Code Mode sends one message, and execute's result stays", async () => {
  const daemon = await standIn((request) => {
    const event = request.params.event as { tool: string }

    return { result: { actions: event.tool === "shell" ? [NUDGE] : [] } }
  })

  try {
    const { hook, sent } = await load({ socket: daemon.socket }, () => {})
    const content = [{ type: "text", text: '{ "exit": 0 }' }]
    const execute = {
      id: "toolu_1",
      tool: "execute",
      sessionID: "ses_1",
      input: { code: 'return await tools.shell({ command: "git commit -m wip" })' },
      status: "completed",
      result: { content: [...content] }
    }

    await hook(commit("completed"))
    await hook(execute)

    expect(sent).toHaveLength(1)
    expect(execute.result.content).toEqual(content)
  } finally {
    await daemon.close()
  }
})

test("the plugin is named after its installation", () => {
  const shipped = "file:///opt/homebrew/share/chauffeur-next/opencode/server.js"

  expect(adapter(shipped).id).toBe("chauffeur-next")
  expect(adapter(import.meta.url).id).toBe("chauffeur")
})
