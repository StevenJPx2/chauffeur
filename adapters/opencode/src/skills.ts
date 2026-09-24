import { Effect } from "effect"
import { Host, type SessionID } from "./host.js"
import type { ContextEffect } from "./protocol.js"

/** Synthetic messages carrying skills record their IDs here. */
export const SKILLS_METADATA_KEY = "chauffeur.skills"

/**
 * Deliver context outside the prompt hook as one synthetic message: the text,
 * then each skill's body in OpenCode's own skill-message form. `steer` reaches
 * the running turn, `resume` wakes an idle agent, and `wait` stays for the
 * next turn.
 */
export function deliverContext(sessionID: SessionID, effect: ContextEffect): Effect.Effect<void, unknown, Host> {
  return Effect.gen(function* () {
    const host = yield* Host
    const catalog = (yield* host.skill.list()).data
    const skills = effect.skills.flatMap((id) => catalog.filter((candidate) => candidate.id === id))

    const parts = [
      ...(effect.text ? [effect.text] : []),
      ...skills.map((skill) => `<skill_content name="${skill.id}">\n# Skill: ${skill.id}\n\n${skill.content}\n</skill_content>`),
    ]

    if (parts.length === 0) return

    yield* host.session.synthetic({
      sessionID,
      text: parts.join("\n\n"),
      description: `Chauffeur ${effect.label}`,
      metadata: { [SKILLS_METADATA_KEY]: skills.map((skill) => skill.id) },
      ...(effect.delivery === "steer" ? { delivery: "steer" as const } : { resume: effect.delivery === "resume" }),
    })
  })
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
