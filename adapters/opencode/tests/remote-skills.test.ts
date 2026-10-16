import { expect, test } from "bun:test"
import { type Plugin, Skill } from "@opencode/plugin/effect"
import { Effect, Schema } from "effect"
import { DaemonError } from "../src/daemon.js"
import { SkillsReply } from "../src/protocol.js"
import { offerRemoteSkills } from "../src/remote-skills.js"
import { fakeHost, install, noDaemon, settle } from "./support.js"

type Editor = Parameters<Parameters<Plugin.Context["skill"]["transform"]>[0]>[0]

function registry(local: ReadonlyArray<Skill.Info>) {
  const transforms: Array<(editor: Editor) => void> = []
  let catalog = new Map<string, Skill.Info>()
  let reloads = 0

  const rebuild = () => {
    catalog = new Map(local.map((skill) => [String(skill.id), skill]))

    const editor = {
      get: (id: string) => catalog.get(id),
      add: (skill: Skill.Info) => { catalog.set(String(skill.id), skill) },
      list: () => [...catalog.values()],
      remove: (id: string) => { catalog.delete(id) },
      update: () => {},
    }

    for (const transform of transforms) transform(editor)
  }

  const host = fakeHost({ skill: {
    transform: (transform: (editor: Editor) => void) => Effect.sync(() => {
      transforms.push(transform)
      rebuild()

      return { dispose: Effect.void }
    }),
    reload: () => Effect.sync(() => { reloads++; rebuild() }),
    list: () => Effect.sync(() => ({ data: [...catalog.values()] })),
  } })

  return { host, current: () => catalog, reloads: () => reloads }
}

function catalog(content: string): SkillsReply {
  return SkillsReply.make({ skills: [
    { id: "review", description: "Review code", content, path: "/cache/review/SKILL.md", source_id: "team", source_url: "https://example.com/", revision: content },
    { id: "extra", description: "Extra work", content, path: "/cache/extra/SKILL.md", source_id: "team", source_url: "https://example.com/", revision: content },
  ], sources: [{ id: "team", state: "ready", revision: content, checked_at: 1000 }] })
}

test("the registry bridge exposes the daemon's catalog before a first prompt and keeps entries already in the editor", async () => {
  const local = Schema.decodeUnknownSync(Skill.Info)({ id: "review", name: "review", content: "local override", path: "/local/review/SKILL.md" })
  const host = registry([local])
  const plugin = await install(offerRemoteSkills("1 hour"), host.host, { ...noDaemon, skills: () => Effect.succeed(catalog("remote")) })

  try {
    expect(host.current().get("review")?.content).toBe("local override")
    expect(host.current().get("extra")?.content).toBe("remote")
    expect(host.reloads()).toBe(1)
  } finally {
    await plugin.close()
  }
})

test("updates reload only changed skills; outages preserve the catalog and removal withdraws it", async () => {
  const host = registry([])
  let offer = catalog("first")
  let down = false
  const daemon = { ...noDaemon, skills: () => down ? Effect.fail(new DaemonError({ message: "offline" })) : Effect.succeed(offer) }
  const plugin = await install(offerRemoteSkills("20 millis"), host.host, daemon)

  try {
    await settle()
    expect(host.reloads()).toBe(1)
    offer = catalog("second")

    for (let attempt = 0; attempt < 20 && host.current().get("extra")?.content !== "second"; attempt++) await settle()

    expect(host.current().get("extra")?.content).toBe("second")
    down = true
    await settle()
    expect(host.current().get("extra")?.content).toBe("second")
    down = false
    offer = { skills: [], sources: [] }

    for (let attempt = 0; attempt < 20 && host.current().has("extra"); attempt++) await settle()

    expect(host.current().size).toBe(0)
  } finally {
    await plugin.close()
  }
})
