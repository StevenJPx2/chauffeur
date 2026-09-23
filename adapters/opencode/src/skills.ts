import type { Plugin } from "@opencode/plugin"

/** Synthetic messages carrying a skill record its ID here. */
export const SKILL_METADATA_KEY = "chauffeur.skill"

type SessionID = Parameters<Plugin.Context["session"]["synthetic"]>[0]["sessionID"]

/**
 * Deliver skills outside the prompt hook as synthetic messages holding each
 * skill's body, in OpenCode's own skill-message form. `steer` reaches the
 * running turn; otherwise the skill waits in history for the next turn.
 */
export async function deliverSkills(ctx: Plugin.Context, sessionID: SessionID, ids: string[], steer: boolean): Promise<void> {
  const catalog = (await ctx.skill.list()).data

  for (const id of ids) {
    const skill = catalog.find((candidate) => candidate.id === id)

    if (!skill) continue

    await ctx.session.synthetic({
      sessionID,
      text: `Chauffeur: the ${skill.id} skill fits this work better than the current approach.\n\n<skill_content name="${skill.id}">\n# Skill: ${skill.id}\n\n${skill.content}\n</skill_content>`,
      description: `Chauffeur skill ${skill.id}`,
      metadata: { [SKILL_METADATA_KEY]: skill.id },
      ...(steer ? { delivery: "steer" as const } : { resume: false }),
    })
  }
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
