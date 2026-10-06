import { Effect } from "effect"
import { DESCRIPTION_BYTES } from "./protocol.js"
import { clipBytes } from "./text.js"

/** Skills warned about per process; more are never tracked. */
const MAX_WARNED = 256

/** How much of the kept text a warning quotes, to show where the cut falls. */
const QUOTED_CODE_POINTS = 60

type DescribedSkill = { readonly id: string; readonly description?: string | undefined }

/** A skill whose description Jev reads only in part. */
export type Clipped = { readonly id: string; readonly bytes: number; readonly endsWith: string }

const warned = new Set<string>()

/** The skills whose description exceeds what a signal carries. */
export function clippedSkills(skills: ReadonlyArray<DescribedSkill>): Clipped[] {
  return skills.flatMap((skill) => {
    const description = skill.description ?? ""
    const bytes = Buffer.byteLength(description, "utf8")

    if (bytes <= DESCRIPTION_BYTES) return []

    const kept = [...clipBytes(description, DESCRIPTION_BYTES)]

    return [{ id: skill.id, bytes, endsWith: kept.slice(-QUOTED_CODE_POINTS).join("") }]
  })
}

/**
 * Log each skill whose description is cut, once per process and description:
 * whatever decides when to use a skill must sit in its first bytes, or the
 * judgment never sees it.
 */
export function warnClippedSkills(skills: ReadonlyArray<DescribedSkill>): Effect.Effect<void> {
  return Effect.forEach(clippedSkills(skills), (skill) => {
    const key = `${skill.id}:${skill.bytes}`

    if (warned.has(key) || warned.size >= MAX_WARNED) return Effect.void

    warned.add(key)

    return Effect.logWarning(
      `chauffeur: skill ${skill.id}'s description is ${skill.bytes} bytes; Chauffeur's judge reads the first ${DESCRIPTION_BYTES}, ending "…${skill.endsWith}". Put what decides when to use it first, or shorten it.`,
    )
  }, { discard: true })
}
