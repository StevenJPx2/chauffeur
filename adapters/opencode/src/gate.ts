import { Rpc } from "@opencode/plugin/effect"
import { Effect, Schema } from "effect"
import { Daemon } from "./daemon.js"
import { Host } from "./host.js"
import { signal, TEXT_CODE_POINTS } from "./protocol.js"
import { clip } from "./text.js"

// sourcefed delivers anyway when the gate is slower than this.
const GATE_TIMEOUT = "3 seconds"

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

const GateInput = Schema.Struct({
  sessionID: Schema.String,
  source: Schema.String,
  kind: Schema.String,
  summary: Schema.String,
  body: Schema.optional(Schema.String),
  actionable: Schema.Boolean,
})

/** Register `chauffeur.gate`: only a confident "no" from the engine withholds an event. */
export const installGate = Effect.gen(function* () {
  const host = yield* Host
  const daemon = yield* Daemon

  yield* host.rpc.register(ChauffeurRpc, {
    gate: (input) =>
      Schema.decodeUnknownEffect(GateInput)(input).pipe(
        Effect.flatMap((event) => daemon.signal(signal(event.sessionID, {
          type: "integration_event",
          source: clip(event.source, TEXT_CODE_POINTS),
          kind: clip(event.kind, TEXT_CODE_POINTS),
          summary: clip(event.summary, TEXT_CODE_POINTS),
          body: clip(event.body ?? "", TEXT_CODE_POINTS),
          actionable: event.actionable,
        }), GATE_TIMEOUT)),
        Effect.map((effects) => ({ deliver: !effects.some((effect) => effect.type === "gate" && !effect.deliver) })),
        // A failed gate delivers, as sourcefed does without Chauffeur.
        Effect.catch((error) => Effect.logError("chauffeur: gate unavailable", error).pipe(Effect.as({ deliver: true }))),
      ),
  }).pipe(Effect.catch((error) => Effect.logError("chauffeur: gate not registered", error)))
})
