export type RpcResponse<T> = {
  id?: number
  result?: T
  error?: string
}

/** `variant` is the host's thinking variant, omitted for the model's default. */
export type ModelRef = { provider: string; model: string; variant?: string }

export type CatalogEntry = { id: string; description: string; bytes: number }

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

/** Where context enters the conversation; every delivery lands at the tail. */
export type Delivery = "prompt" | "steer" | "resume" | "wait"

/** Host actions. None names a capability, so new capabilities need no adapter change. */
export type Effect =
  | { type: "permission"; agent_id: string; decision: "allow" | "deny" | "ask"; message: string | null }
  | { type: "model"; agent_id: string; model: ModelRef | null }
  | { type: "tools"; agent_id: string; hide: string[]; reveal: string[] }
  | { type: "context"; agent_id: string; delivery: Delivery; label: string; skills: string[]; text: string | null }
  | { type: "gate"; agent_id: string; deliver: boolean }

export type ContextEffect = Extract<Effect, { type: "context" }>
