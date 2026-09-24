import { type Plugin, Rpc } from "@opencode/plugin"
import type { DaemonBridge } from "./daemon.js"
import { signal } from "./model-router.js"
import { clip } from "./text.js"

const TEXT_CODE_POINTS = 512
// sourcefed delivers anyway when the gate is slower than this.
const GATE_TIMEOUT_MS = 3_000

/**
 * The contract other plugins call: sourcefed asks `chauffeur.gate` before it
 * delivers a monitor event into a session in this process. Plain JSON Schema,
 * so a caller can declare the same definition without depending on Chauffeur.
 */
export const ChauffeurRpc = Rpc.define({
  id: "chauffeur",
  methods: {
    gate: {
      input: {
        type: "object",
        properties: {
          sessionID: { type: "string" },
          source: { type: "string" },
          kind: { type: "string" },
          summary: { type: "string" },
          body: { type: "string" },
          actionable: { type: "boolean" },
        },
        required: ["sessionID", "source", "kind", "summary", "actionable"],
      },
      output: {
        type: "object",
        properties: { deliver: { type: "boolean" } },
        required: ["deliver"],
      },
    },
  },
  events: {},
})

type GateInput = { sessionID: string; source: string; kind: string; summary: string; body?: string; actionable: boolean }

/** Register `chauffeur.gate`: only a confident "no" from the engine withholds an event. */
export async function installGate(ctx: Plugin.Context, daemon: DaemonBridge): Promise<() => Promise<void>> {
  const registration = await ctx.rpc.register(ChauffeurRpc, {
    gate: async (input) => {
      const event = input as GateInput

      try {
        const effects = await daemon.signal(signal(event.sessionID, {
          type: "integration_event",
          source: clip(event.source, TEXT_CODE_POINTS),
          kind: clip(event.kind, TEXT_CODE_POINTS),
          summary: clip(event.summary, TEXT_CODE_POINTS),
          body: clip(event.body ?? "", TEXT_CODE_POINTS),
          actionable: event.actionable,
        }), GATE_TIMEOUT_MS)

        return { deliver: !effects.some((effect) => effect.type === "gate" && !effect.deliver) }
      } catch (error) {
        // A failed gate delivers, as sourcefed does without Chauffeur.
        console.error(`[chauffeur] gate unavailable: ${String(error)}`)
        return { deliver: true }
      }
    },
  })

  return () => registration.dispose()
}
