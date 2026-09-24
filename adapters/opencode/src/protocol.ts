import { Schema } from "effect"
import { clip } from "./text.js"

/** Signal text fields, in code points: 4 bytes each stays within the engine's 2,048-byte text bound. */
export const TEXT_CODE_POINTS = 512

// 100 code points stays within the engine's 400-byte description bound.
const DESCRIPTION_CODE_POINTS = 100

/** A model as Chauffeur names it; `variant` is the host's thinking variant, absent for the default. */
const ModelRef = Schema.Struct({
  provider: Schema.String,
  model: Schema.String,
  variant: Schema.optional(Schema.String),
})

export type ModelRef = typeof ModelRef.Type

export type CatalogEntry = { id: string; description: string; bytes: number }

/** A catalog entry within the engine's bounds; `bytes` is the size of `content`, what attaching it adds. */
export function catalogEntry(id: string, description: string, content: string): CatalogEntry {
  return {
    id: clip(id, TEXT_CODE_POINTS),
    description: clip(description, DESCRIPTION_CODE_POINTS),
    bytes: Buffer.byteLength(content, "utf8"),
  }
}

/** A Code Mode namespace: its size and the host's best matches for the request. */
export type CodeModeNamespace = { name: string; size: number; tools: CatalogEntry[] }

export type Resource = { requested: string; resolved: string }

export type SignalKind =
  | {
    type: "permission_request"
    action: string
    resources: Resource[]
    request: string
    workspace: string
    user_requests: string[]
  }
  | {
    type: "user_message"
    text: string
    first_in_context: boolean
    skills: CatalogEntry[]
    tools: CatalogEntry[]
    model: ModelRef | null
    code_mode: CodeModeNamespace[]
  }
  | { type: "tool_result"; tool: string; ok: boolean; workspace: string; input: string; error: string; user_request: string; evidence: string; candidates: CatalogEntry[] }
  | { type: "turn_end"; workspace: string; user_request: string }
  | {
    type: "model_error"
    model: ModelRef
    error_type: string
    status: number | null
    message: string
    tool_executed: boolean
    available: Array<{ model: ModelRef; usable: boolean }>
  }
  | { type: "model_succeeded"; model: ModelRef }
  | { type: "integration_event"; source: string; kind: string; summary: string; body: string; actionable: boolean }

export type Signal = { agent_id: string; at: number; kind: SignalKind }

export function signal(agentID: string, kind: SignalKind): Signal {
  return { agent_id: agentID, at: Math.floor(Date.now() / 1_000), kind }
}

/** Where context enters the conversation; every delivery lands at the tail. */
const Delivery = Schema.Literals(["prompt", "steer", "resume", "wait"])

/** Host actions. None names a capability, so new capabilities need no adapter change. */
const HostEffect = Schema.Union([
  Schema.Struct({
    type: Schema.Literal("permission"),
    agent_id: Schema.String,
    decision: Schema.Literals(["allow", "deny", "ask"]),
    message: Schema.NullOr(Schema.String),
  }),
  Schema.Struct({ type: Schema.Literal("model"), agent_id: Schema.String, model: Schema.NullOr(ModelRef) }),
  Schema.Struct({
    type: Schema.Literal("tools"),
    agent_id: Schema.String,
    hide: Schema.Array(Schema.String),
    reveal: Schema.Array(Schema.String),
  }),
  Schema.Struct({
    type: Schema.Literal("context"),
    agent_id: Schema.String,
    delivery: Delivery,
    label: Schema.String,
    skills: Schema.Array(Schema.String),
    text: Schema.NullOr(Schema.String),
  }),
  Schema.Struct({ type: Schema.Literal("gate"), agent_id: Schema.String, deliver: Schema.Boolean }),
])

export type HostEffect = typeof HostEffect.Type

export type ContextEffect = Extract<HostEffect, { readonly type: "context" }>

export const SignalReply = Schema.Struct({ effects: Schema.Array(HostEffect) })

export const RpcReply = Schema.Struct({
  result: Schema.optional(Schema.Unknown),
  error: Schema.optional(Schema.String),
})
