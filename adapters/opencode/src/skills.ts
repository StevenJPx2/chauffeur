import { Effect, Schema } from "effect"
import { currentContext, type HistoryMessage, Host, type SessionID } from "./host.js"
import type { ContextEffect } from "./protocol.js"
import { fill, type HostTexts } from "./texts.js"

/** Synthetic messages and `ask_chauffeur` results carrying skills record their IDs here. */
export const SKILLS_METADATA_KEY = "chauffeur.skills"

const isStringList = Schema.is(Schema.Array(Schema.String))

type Metadata = Extract<HistoryMessage, { readonly type: "synthetic" }>["metadata"]

/**
 * Skills already in a context: attached to a user message, delivered by
 * Chauffeur in a message or an `ask_chauffeur` reply, or loaded by the host.
 */
export function skillsInContext(context: ReadonlyArray<HistoryMessage>): Set<string> {
  const recorded = (metadata: Metadata): ReadonlyArray<string> => {
    const ids = metadata?.[SKILLS_METADATA_KEY]

    return isStringList(ids) ? ids : []
  }

  return new Set(context.flatMap((message): ReadonlyArray<string> => {
    switch (message.type) {
      case "user": return (message.skills ?? []).map((skill) => String(skill.id))
      case "skill": return [String(message.skill)]
      case "synthetic": return recorded(message.metadata)
      case "assistant": return message.content.flatMap((part) => (part.type === "tool" && "metadata" in part.state ? recorded(part.state.metadata) : []))
      default: return []
    }
  }))
}

/**
 * Whether the host keeps its own skill tool and skill list: when skill
 * exposure is off (`CHAUFFEUR_DISABLE`), or when `CHAUFFEUR_HOST_SKILLS=keep`
 * keeps them beside the skills Chauffeur attaches.
 */
export function hostLoadsSkills(env: Record<string, string | undefined> = process.env): boolean {
  const disabled = (env.CHAUFFEUR_DISABLE ?? "").split(",").map((id) => id.trim())

  return disabled.includes("skill-exposure") || env.CHAUFFEUR_HOST_SKILLS === "keep"
}

/**
 * Deliver context outside the prompt hook as one synthetic message: the text,
 * then the body of each skill not already in the context, in OpenCode's own
 * skill-message form. `steer` reaches the running turn, `resume` wakes an
 * idle agent, and `wait` stays for the next turn.
 */
export function deliverContext(sessionID: SessionID, effect: ContextEffect, texts: HostTexts): Effect.Effect<void, unknown, Host> {
  return Effect.gen(function* () {
    const host = yield* Host
    const present = effect.skills.length === 0 ? new Set<string>() : skillsInContext(currentContext(yield* host.session.context({ sessionID })))
    const { text, skills } = yield* renderContext({ ...effect, skills: effect.skills.filter((skill) => !present.has(skill)) })

    if (text === "") return

    yield* host.session.synthetic({
      sessionID,
      text,
      description: fill(texts.labels.context, { label: effect.label }),
      metadata: { [SKILLS_METADATA_KEY]: skills },
      ...(effect.delivery === "steer" ? { delivery: "steer" as const } : { resume: effect.delivery === "resume" }),
    })
  })
}

/** A context effect's text, then each skill's body in OpenCode's own skill-message form. */
export function renderContext(effect: ContextEffect): Effect.Effect<{ text: string; skills: string[] }, unknown, Host> {
  return Effect.gen(function* () {
    const host = yield* Host
    const catalog = (yield* host.skill.list()).data
    const skills = effect.skills.flatMap((id) => catalog.filter((candidate) => candidate.id === id))

    const parts = [
      ...(effect.text ? [effect.text] : []),
      ...skills.map((skill) => `<skill_content name="${skill.id}">\n# Skill: ${skill.id}\n\n${skill.content}${baseDirectory(skill.path, skill.content)}\n</skill_content>`),
    ]

    return { text: parts.join("\n\n"), skills: skills.map((skill) => skill.id) }
  })
}

const BASE_DIRECTORY = "Base directory for this skill:"

/**
 * Where a skill's relative paths start, as OpenCode states it when it loads a
 * skill itself; without it the agent cannot find the skill's scripts or
 * references. Nothing when the body already says, as a remote skill's does.
 */
function baseDirectory(path: string | undefined, content: string): string {
  if (!path?.startsWith("/") || content.includes(BASE_DIRECTORY)) return ""

  const directory = path.endsWith("/SKILL.md") ? path.slice(0, -"/SKILL.md".length) : path

  return `\n\n${BASE_DIRECTORY} ${directory}\nRelative paths in this skill (e.g., scripts/, references/) are relative to this base directory.`
}

/**
 * Chauffeur owns skill loading, so every agent is denied the host's `skill`
 * tool. OpenCode then leaves both that tool and its skill list out of each
 * request; skills Chauffeur attaches to prompts still resolve. The rule lives
 * only in this plugin, so disabling Chauffeur restores both.
 */
export const claimSkillLoading = Effect.gen(function* () {
  const host = yield* Host

  yield* host.agent.transform((agents) => {
    for (const agent of agents.list()) {
      agents.update(String(agent.id), (info) => {
        info.permissions.push({ action: "skill", resource: "*", effect: "deny" })
      })
    }
  })
})
