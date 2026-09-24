import { expect, test } from "bun:test"
import { Effect } from "effect"
import { type DaemonClient, DaemonError } from "../src/daemon.js"
import { installGate } from "../src/gate.js"
import type { HostEffect } from "../src/protocol.js"
import { fakeHost, install } from "./support.js"

type Gate = (input: Readonly<Record<string, string | boolean>>) => Effect.Effect<{ readonly deliver: boolean }>

async function gate(daemon: DaemonClient, input: Readonly<Record<string, string | boolean>>): Promise<boolean> {
  const handlers = new Map<"gate", Gate>()

  const host = fakeHost({
    rpc: {
      register: (_: string, registered: { readonly gate: Gate }) => Effect.sync(() => {
        handlers.set("gate", registered.gate)

        return { dispose: Effect.void }
      }),
    },
  })

  const plugin = await install(installGate, host, daemon)
  const registered = handlers.get("gate")
  const reply = registered ? await Effect.runPromise(registered(input)) : { deliver: true }

  await plugin.close()

  return reply.deliver
}

const event = { sessionID: "ses_gate", source: "github", kind: "ci", summary: "CI passed", actionable: false }

function answering(effects: ReadonlyArray<HostEffect>): DaemonClient {
  return { signal: () => Effect.succeed(effects) }
}

test("only a gate effect that withholds keeps an event out", async () => {
  expect(await gate(answering([{ type: "gate", agent_id: "ses_gate", deliver: false }]), event)).toBe(false)
  expect(await gate(answering([]), event)).toBe(true)
})

test("an invalid request or an unavailable engine delivers", async () => {
  expect(await gate(answering([{ type: "gate", agent_id: "ses_gate", deliver: false }]), { sessionID: "ses_gate" })).toBe(true)
  expect(await gate({ signal: () => Effect.fail(new DaemonError({ message: "daemon down" })) }, event)).toBe(true)
})
