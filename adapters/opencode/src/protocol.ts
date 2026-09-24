export type RpcResponse<T> = {
  id?: number
  result?: T
  error?: string
}

/** `variant` is the host's thinking variant, omitted for the model's default. */
export type ModelRef = { provider: string; model: string; variant?: string }

export type CatalogEntry = { id: string; description: string; bytes: number }

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
    code_mode: CatalogEntry[]
  }
  | { type: "tool_result"; tool: string; ok: boolean; input: string; error: string }
  | { type: "turn_end" }
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

export type Effect =
  | { type: "permission"; agent_id: string; decision: "allow" | "deny" | "ask"; message: string | null }
  | { type: "attach_skills"; agent_id: string; skills: string[] }
  | { type: "remind"; agent_id: string; rule_id: string; text: string }
  | { type: "nudge"; agent_id: string; tool: string; text: string }
  | { type: "hide_tools"; agent_id: string; tools: string[] }
  | { type: "reveal_tools"; agent_id: string; tools: string[] }
  | { type: "surface_tools"; agent_id: string; namespaces: string[] }
  | { type: "switch_model"; agent_id: string; model: ModelRef }
  | { type: "keep_model"; agent_id: string }
  | { type: "withhold_event"; agent_id: string }
