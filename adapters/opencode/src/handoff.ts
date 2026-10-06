import { readdirSync, readFileSync } from "node:fs"
import { join } from "node:path"
import { Skill } from "@opencode/plugin/effect"
import { Effect, Option, Schema, type Scope } from "effect"
import { Host } from "./host.js"
import { packageSkills } from "./package.js"

const decodeSkill = Schema.decodeUnknownOption(Skill.Info)

/**
 * A `SKILL.md` as OpenCode loads it: `name` and `description` from one-line
 * frontmatter fields, and the body after the frontmatter as its content.
 * Undefined without a name, or with a field written as a YAML block.
 */
export function parseSkill(text: string, path: string): Skill.Info | undefined {
  const match = /^---\n([\s\S]*?)\n---\n?/.exec(text)

  if (!match?.[1]) return undefined

  const fields = new Map(match[1].split("\n").flatMap((line) => {
    const field = /^([a-z][\w-]*):\s*(.*)$/.exec(line)

    return field?.[1] && field[2] ? [[field[1], field[2].replace(/^(["'])(.*)\1$/, "$2")] as const] : []
  }))

  const name = fields.get("name")
  // A YAML block (`>-`, `|`) spans lines this reader does not follow.
  const block = [...fields.values()].some((value) => /^[>|][-+]?$/.test(value))

  return name === undefined || block
    ? undefined
    : Option.getOrUndefined(decodeSkill({ id: name, name, description: fields.get("description"), path, content: text.slice(match[0].length).trim() }))
}

/** Every `<folder>/<skill>/SKILL.md` that parses. */
export function handoffSkills(folder: string): Skill.Info[] {
  const entries = (() => {
    try {
      return readdirSync(folder, { withFileTypes: true })
    } catch {
      return []
    }
  })()

  return entries.filter((entry) => entry.isDirectory()).flatMap((entry) => {
    const path = join(folder, entry.name, "SKILL.md")

    try {
      const skill = parseSkill(readFileSync(path, "utf8"), path)

      return skill ? [skill] : []
    } catch {
      return []
    }
  })
}

/**
 * Offer the skills Chauffeur hands over (`skills/handoff`) when the plugin runs
 * from its package. A skill the user already has under the same name wins.
 */
export const installHandoffSkills: Effect.Effect<void, never, Host | Scope.Scope> = Effect.gen(function* () {
  const host = yield* Host
  const folder = packageSkills()
  const skills = folder === undefined ? [] : handoffSkills(join(folder, "handoff"))

  if (skills.length === 0) return

  yield* host.skill.transform((editor) => {
    for (const skill of skills) {
      if (!editor.get(skill.id)) editor.add(skill)
    }
  })
})
