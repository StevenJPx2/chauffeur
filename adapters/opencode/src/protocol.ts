export type Target = {
  kind: "opencode-session"
  id: string
}

export type ToolCall = {
  name: string
  summary: string
  at: number
}

export type AgentContext = {
  agent_id: string
  status: string
  source: string
  tool_history: ToolCall[]
  notifications: Notice[]
  hooks: string[]
  idle_at: number
}

export type Notice = {
  source: string
  title: string
  body: string
  acknowledged: boolean
}

export type QueuedReminder = {
  id: string
  target: Target
  reminder: {
    agent_id: string
    rule_id: string
    urgency: "routine" | "important" | "urgent"
    text: string
  }
  queued_at: number
}

export type SkillContext = {
  event: string
  action: string
  agent_id: string
  occurred_at: number
  state: string
  evidence: Record<string, boolean>
}

export type SkillEffect = "allow" | "deny" | "ask" | "prompt" | "remind"

export type SkillResult = {
  skill_id: string
  status: "unmatched" | "missing_evidence" | "cooldown" | "evaluated" | "judge_failure"
  effect: SkillEffect | null
  message: string | null
  reminder: string | null
  missing_evidence: string[]
  confidence: number | null
  branch: "positive" | "negative" | "uncertain" | null
}

export type EventFrame =
  | { type: "subscribed"; target: Target }
  | { type: "event"; target: Target; reminders: QueuedReminder[] }
  | { type: "heartbeat" }

export type RpcResponse<T> = {
  id?: number
  result?: T
  error?: string
}
