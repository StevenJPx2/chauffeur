// The development loop: build the Rust side, serve the page with Vite, and
// rebuild the Rust side whenever its source changes. Vite reloads the page
// when the rebuilt module lands.

import { type ChildProcess, spawn } from "node:child_process"
import { watch } from "node:fs"
import { fileURLToPath } from "node:url"

const root = fileURLToPath(new URL("..", import.meta.url))
const build = ["build", "wasm", "--target", "web", "--dev", "--out-dir", "../src/wasm", "--no-pack"]

/** One build at a time; a change during a build queues exactly one more. */
let building: Promise<void> | undefined
let queued = false

function rebuild(): Promise<void> {
  if (building !== undefined) {
    queued = true
    return building
  }

  building = new Promise((resolve) => {
    const child = spawn("wasm-pack", build, { cwd: root, stdio: "inherit" })

    child.once("exit", (code) => {
      if (code !== 0) console.error("[playground] Rust build failed; the page keeps the last good build")
      building = undefined
      resolve()

      if (queued) {
        queued = false
        void rebuild()
      }
    })
  })

  return building
}

function start(command: string, args: ReadonlyArray<string>): ChildProcess {
  return spawn(command, args, { cwd: root, stdio: "inherit" })
}

// The page imports the built module, so serve it only once it exists.
await rebuild()

const vite = start("bunx", ["vite", ...process.argv.slice(2)])
// The bridge and the crates it compiles in.
const watchers = [`${root}wasm`, `${root}../crates`].map((dir) =>
  watch(dir, { recursive: true }, (_event, file) => {
    if (file !== null && (file.endsWith(".rs") || file.endsWith("Cargo.toml"))) void rebuild()
  }),
)

/** Stop the watchers and Vite together, however the loop ends. */
function stop(): void {
  for (const watcher of watchers) watcher.close()
  vite.kill()
  process.exit(0)
}

process.once("SIGINT", stop)
process.once("SIGTERM", stop)
vite.once("exit", stop)
