import { Skill } from "@opencode/plugin/effect"
import { type Duration, Effect, Schedule, Schema, type Scope } from "effect"
import { Daemon } from "./daemon.js"
import { Host } from "./host.js"
import type { SkillsReply } from "./protocol.js"

/**
 * Only a registry bridge. The shared Rust service owns parsing, upstream I/O, and freshness.
 * OpenCode 2.0.28's transform editor sees only plugin-registered skills, not discovered
 * SKILL.md folders, so a configured remote skill shadows a discovered one of the same name.
 */
export function offerRemoteSkills(refresh: Duration.Input = "5 seconds"): Effect.Effect<void, never, Host | Daemon | Scope.Scope> {
  return Effect.gen(function* () {
    const host = yield* Host
    const daemon = yield* Daemon
    let offer: ReadonlyArray<Skill.Info> = []
    let current = "[]"
    let warnings = "[]"

    yield* host.skill.transform((editor) => {
      for (const skill of offer) {
        if (!editor.get(skill.id)) editor.add(skill)
      }
    })

    const sync = Effect.gen(function* () {
      const catalog = yield* daemon.skills()
      const next = JSON.stringify(catalog.skills)
      const problems = catalog.sources.filter((source) => source.state !== "ready")
      const warningKey = JSON.stringify(problems)

      if (warningKey !== warnings) {
        for (const source of problems) yield* Effect.logWarning(`chauffeur: skill source ${source.id} ${source.state}: ${source.error}`)

        warnings = warningKey
      }

      if (next === current) return

      offer = yield* Effect.forEach(catalog.skills, hostSkill)
      yield* host.skill.reload()
      // Reload is lazy; publish the new registry now, not only after another prompt.
      yield* host.skill.list()
      current = next
    }).pipe(Effect.catch((error) => Effect.logWarning("chauffeur: remote skill catalog unavailable; keeping the previous offer", error)))

    // A cold catalog is initialized in the daemon, so the first prompt can already use it.
    yield* sync
    yield* sync.pipe(Effect.repeat(Schedule.spaced(refresh)), Effect.forkScoped)
  })
}

function hostSkill(skill: SkillsReply["skills"][number]) {
  return Schema.decodeUnknownEffect(Skill.Info)({
    id: skill.id,
    name: skill.id,
    description: skill.description,
    path: skill.path,
    content: skill.content,
  })
}

export const installRemoteSkills = offerRemoteSkills()
