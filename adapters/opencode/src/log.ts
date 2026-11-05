import { appendFileSync, mkdirSync } from "node:fs"
import { homedir } from "node:os"
import { join } from "node:path"

type Env = Readonly<Record<string, string | undefined>>

/** `opencode.log` in the installation's state folder, beside its audit log. */
export function logPath(name: string, env: Env): string {
  const state = env.XDG_STATE_HOME || join(env.HOME || homedir(), ".local/state")

  return join(state, name, "opencode.log")
}

/** Appends a line with the time; a log that cannot be written is let go. */
export function fileLog(path: string, now = () => new Date()) {
  return (message: string) => {
    try {
      mkdirSync(join(path, ".."), { recursive: true })
      appendFileSync(path, `${now().toISOString()} ${message}\n`)
    } catch {}
  }
}
