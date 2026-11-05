import { expect, test } from "bun:test"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { pathToFileURL } from "node:url"
import { emit, installation, socketPath } from "../src/daemon.ts"
import { NUDGE, standIn } from "./stand-in.ts"

const EVENT = { event: "tool.after", session: "s1", tool: "shell" }

test("the folder the adapter ships in names the installation", () => {
  const shipped = "/opt/homebrew/share/chauffeur-next/opencode/server.js"
  const source = "/src/chauffeur/adapters/opencode/src/index.ts"

  expect(installation(pathToFileURL(shipped).href)).toBe("chauffeur-next")
  expect(installation(pathToFileURL(source).href)).toBe("chauffeur")
})

test("the socket is the option's, then $CHAUFFEUR_SOCKET, then the runtime folder's", () => {
  const env = { CHAUFFEUR_SOCKET: "/env/s", XDG_RUNTIME_DIR: "/run/user/1" }

  expect(socketPath("chauffeur-next", env, "/option/s")).toBe("/option/s")
  expect(socketPath("chauffeur-next", env)).toBe("/env/s")

  expect(socketPath("chauffeur-next", { ...env, CHAUFFEUR_SOCKET: "" })).toBe(
    "/run/user/1/chauffeur-next/daemon.sock"
  )

  expect(socketPath("chauffeur", {})).toBe(join(tmpdir(), "chauffeur/daemon.sock"))
})

test("emit sends chauffeur.emit with the deadline and returns the actions", async () => {
  const daemon = await standIn(() => ({ result: { actions: [NUDGE] } }))

  try {
    expect(await emit(daemon.socket, EVENT, 1500)).toEqual([NUDGE])

    expect(daemon.requests).toEqual([
      expect.objectContaining({
        method: "chauffeur.emit",
        params: { event: EVENT, deadline_ms: 1500 }
      })
    ])
  } finally {
    await daemon.close()
  }
})

test("emit fails with no daemon, an error, a late answer or no actions", async () => {
  const missing = join(tmpdir(), "chauffeur-missing", "s")
  await expect(emit(missing, EVENT, 100)).rejects.toThrow()

  const refusing = await standIn(() => ({ error: { code: -32602, message: "bad" } }))
  const silent = await standIn(() => undefined)
  const empty = await standIn(() => ({ result: {} }))

  try {
    await expect(emit(refusing.socket, EVENT, 100)).rejects.toThrow("bad")
    await expect(emit(silent.socket, EVENT, 10)).rejects.toThrow(
      "no answer within 510 ms"
    )
    await expect(emit(empty.socket, EVENT, 100)).rejects.toThrow("no actions")
  } finally {
    await Promise.all([refusing.close(), silent.close(), empty.close()])
  }
})
