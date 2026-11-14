import { tmpdir } from "node:os"
import { join } from "node:path"

type Env = Readonly<Record<string, string | undefined>>

/** Where to find the daemon: a socket, or an installation's name, such as `chauffeur-next`. */
export type Where = { readonly socket?: string; readonly name?: string }

/**
 * The daemon's socket: the `socket` option; else the `name` option's
 * `<name>/daemon.sock` in the runtime folder, so an installation asked for
 * by name wins over the environment; else `$CHAUFFEUR_SOCKET`; else
 * `chauffeur/daemon.sock`, as the daemon finds it. The runtime folder is
 * `$XDG_RUNTIME_DIR`, or the temporary folder when that is unset.
 */
export function socketPath(where: Where = {}, env: Env = process.env): string {
  const set = (value: string | undefined) => (value ? value : undefined)
  const runtime = set(env.XDG_RUNTIME_DIR) ?? tmpdir()
  const named = where.name ? join(runtime, where.name, "daemon.sock") : undefined

  return (
    where.socket ??
    named ??
    set(env.CHAUFFEUR_SOCKET) ??
    join(runtime, "chauffeur", "daemon.sock")
  )
}
