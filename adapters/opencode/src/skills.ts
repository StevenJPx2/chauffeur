import type { Plugin } from "@opencode/plugin"
import type { ContextEffect } from "./protocol.js"

/** Synthetic messages carrying skills record their IDs here. */
export const SKILLS_METADATA_KEY = "chauffeur.skills"

type SessionID = Parameters<Plugin.Context["session"]["synthetic"]>[0]["sessionID"]

/**
 * Deliver context outside the prompt hook as one synthetic message: the text,
 * then each skill's body in OpenCode's own skill-message form. `steer` reaches
 * the running turn, `resume` wakes an idle agent, and `wait` stays for the
 * next turn.
 */
export async function deliverContext(ctx: Plugin.Context, sessionID: SessionID, effect: ContextEffect): Promise<void> {
  const catalog = (await ctx.skill.list()).data
  const skills = effect.skills.flatMap((id) => catalog.filter((candidate) => candidate.id === id))
  const parts = [
    ...(effect.text ? [effect.text] : []),
    ...skills.map((skill) => `<skill_content name="${skill.id}">\n# Skill: ${skill.id}\n\n${skill.content}\n</skill_content>`),
  ]

  if (parts.length === 0) return

  await ctx.session.synthetic({
    sessionID,
    text: parts.join("\n\n"),
    description: `Chauffeur ${effect.label}`,
    metadata: { [SKILLS_METADATA_KEY]: skills.map((skill) => skill.id) },
    ...(effect.delivery === "steer" ? { delivery: "steer" as const } : { resume: effect.delivery === "resume" }),
  })
}

/**
 * Chauffeur owns skill loading, so every agent is denied the host's `skill`
 * tool. OpenCode then leaves both that tool and its skill list out of each
 * request; skills Chauffeur attaches to prompts still resolve. The rule lives
 * only in this plugin, so disabling Chauffeur restores both.
 */
export async function claimSkillLoading(ctx: Plugin.Context): Promise<() => Promise<void>> {
  const registration = await ctx.agent.transform((agents) => {
    for (const agent of agents.list()) {
      agents.update(String(agent.id), (info) => {
        info.permissions.push({ action: "skill", resource: "*", effect: "deny" })
      })
    }
  })

  return () => registration.dispose()
}
