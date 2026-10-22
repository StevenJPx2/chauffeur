import assert from "node:assert/strict"
import { Effect } from "effect"
import { Daemon } from "../../src/daemon.js"
import { PLUGIN_VERSION } from "../../src/package.js"
import { installPermission } from "../../src/permission.js"
import { SHIPPED_TEXTS } from "../../src/texts.js"
import { fakeHost, Hooks, install } from "../support.js"

const sessionID = "ses_starvation"

const controllers = Array.from({ length: 4 }, () => new AbortController())

const methods = new Set<string>()

const server = Bun.serve({
  hostname: "127.0.0.1",
  port: 0,
  idleTimeout: 0,
  async fetch(request) {
    if (new URL(request.url).pathname === "/events") {
      return new Response(new ReadableStream({
        start(controller) {
          controller.enqueue(new TextEncoder().encode("data: subscribed\n\n"))
        },
      }), { headers: { "content-type": "text/event-stream" } })
    }

    const wire = await request.json()

    methods.add(wire.method)

    switch (wire.method) {
      case "health": return Response.json({ result: { ok: true, version: PLUGIN_VERSION } })
      case "texts": return Response.json({ result: SHIPPED_TEXTS })
      case "skills": return Response.json({ result: { skills: [], sources: [] } })
      case "rulebooks": return Response.json({ result: { rulebooks: [] } })
      case "signal": return Response.json({ result: { effects: [{ type: "permission", agent_id: sessionID, decision: "allow", message: null }] } })
      default: throw new Error(`unexpected RPC ${wire.method}`)
    }
  },
})

try {
  const base = `http://127.0.0.1:${server.port}`

  process.env.CHAUFFEUR_DAEMON_URL = base

  const streams = await Promise.all(controllers.map((controller) => fetch(`${base}/events`, { signal: controller.signal })))

  // Prove this subprocess's pool is full, rather than testing under an idle pool.
  await assert.rejects(fetch(`${base}/control`, { signal: AbortSignal.timeout(100) }))

  const daemon = await Effect.runPromise(Daemon.connect.pipe(Effect.timeout("1 second")))

  const hooks = new Hooks()

  const host = fakeHost({
    location: { project: { canonical: "/probe" } },
    permission: { hook: hooks.register },
    session: { context: () => Effect.succeed([{ type: "user", text: "Inspect the repository", metadata: {} }]) },
  })

  const plugin = await install(installPermission, host, daemon)

  try {
    const event = { sessionID, action: "shell", resources: ["git status"], effect: "ask" }

    await hooks.emit("evaluate", event)
    assert.equal(event.effect, "allow")
    await Effect.runPromise(Effect.all([daemon.texts("opencode"), daemon.skills(), daemon.rulebooks("/probe")]).pipe(Effect.timeout("1 second")))
    assert.deepEqual([...methods].sort(), ["health", "rulebooks", "signal", "skills", "texts"])
    assert.equal(streams.length, 4)
    console.log("permission and background RPCs succeeded with the fetch pool saturated")
  } finally {
    await plugin.close()
  }
} finally {
  for (const controller of controllers) controller.abort()
  await server.stop(true)
}
