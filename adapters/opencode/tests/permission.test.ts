import { expect, test } from "bun:test"
import { Effect } from "effect"
import { type DaemonClient, DaemonError } from "../src/daemon.js"
import { installPermission } from "../src/permission.js"
import type { HostEffect, Signal } from "../src/protocol.js"
import { fakeHost, Hooks, install } from "./support.js"

const sessionID = "ses_permission"

type Evaluation = { sessionID: string; action: string; resources: string[]; effect: string; message?: string }

function permissionHost(hooks: Hooks) {
  return fakeHost({
    location: { project: { canonical: "/projects/chauffeur" } },
    permission: { hook: hooks.register },
    session: {
      context: () => Effect.succeed([
        { type: "user", text: "Fix the failing test", metadata: {} },
        { type: "user", text: "CI failed", metadata: { sourcefed: { kind: "ci" } } },
      ]),
    },
  })
}

function replying(sent: Signal[], effects: ReadonlyArray<HostEffect>): DaemonClient {
  return { signal: (value) => Effect.sync(() => {
    sent.push(value)

    return effects
  }) }
}

async function evaluate(daemon: DaemonClient, event: Evaluation): Promise<Evaluation> {
  const hooks = new Hooks()
  const plugin = await install(installPermission, permissionHost(hooks), daemon)

  await hooks.emit("evaluate", event)
  await plugin.close()

  return event
}

test("the engine's decision answers the request, with resolved paths and only the user's words", async () => {
  const sent: Signal[] = []

  const event = await evaluate(
    replying(sent, [{ type: "permission", agent_id: sessionID, decision: "allow", message: "Serves the task." }]),
    { sessionID, action: "edit", resources: ["src/index.ts"], effect: "ask" },
  )

  expect(event).toMatchObject({ effect: "allow", message: "Serves the task." })
  expect(sent[0]?.kind).toMatchObject({
    type: "permission_request",
    resources: [{ requested: "src/index.ts", resolved: "/projects/chauffeur/src/index.ts" }],
    user_requests: ["Fix the failing test"],
  })
})

test("a host denial stands without asking the engine", async () => {
  const sent: Signal[] = []
  const event = await evaluate(replying(sent, []), { sessionID, action: "shell", resources: ["rm -rf /"], effect: "deny" })

  expect(event.effect).toBe("deny")
  expect(sent).toHaveLength(0)
})

test("an engine failure or an oversized request asks", async () => {
  const failed = await evaluate(
    { signal: () => Effect.fail(new DaemonError({ message: "daemon down" })) },
    { sessionID, action: "shell", resources: ["ls"], effect: "allow" },
  )

  const oversized = await evaluate(replying([], []), {
    sessionID, action: "edit", resources: Array.from({ length: 33 }, (_, index) => `file-${index}.ts`), effect: "allow",
  })

  expect(failed).toMatchObject({ effect: "ask", message: expect.stringContaining("daemon down") })
  expect(oversized).toMatchObject({ effect: "ask", message: "Chauffeur could not evaluate this request: more than 32 resources" })
})
