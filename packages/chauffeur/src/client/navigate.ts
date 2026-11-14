import { Connection, type CallOptions } from "./connection.ts"
import { socketPath, type Where } from "./socket.ts"
import { NavigateError, type NavigateRequest, type Navigated } from "./wire.ts"

/** How one navigation is sent: its signal, its events, and where the daemon is. */
export type NavigateOptions = CallOptions & Where

/**
 * Sends one navigation on a connection of its own, and closes it when the
 * result arrives or the signal aborts, so a script exits as soon as it is
 * done. Rejects with a `NavigateError`, or the signal's reason.
 */
export async function navigate<R extends NavigateRequest>(
  request: R,
  options: NavigateOptions = {}
): Promise<Navigated<R>> {
  const connection = await Connection.open(socketPath(options))

  try {
    return (await connection.call(request, options)) as Navigated<R>
  } finally {
    connection.close()
  }
}

/** A connection kept for many navigations, opened again after the daemon restarts. */
export type Chauffeur = {
  navigate<R extends NavigateRequest>(
    request: R,
    options?: CallOptions
  ): Promise<Navigated<R>>
  close(): void
}

/** A client that keeps one connection to the daemon until `close`. */
export function connect(where: Where = {}): Chauffeur {
  const path = socketPath(where)
  let open: Promise<Connection> | undefined
  let closed = false

  // Calls at once share one attempt; a closed or failed one is replaced once.
  // A connection that opens after `close` is closed at once.
  const connection = async (): Promise<Connection> => {
    const current = (open ??= Connection.open(path))
    const live = await current.catch(() => undefined)

    if (closed) {
      live?.close()
      throw new NavigateError("unavailable", "the client is closed")
    }

    if (live && !live.closed) return live

    if (open === current) open = Connection.open(path)

    const fresh = await open

    if (closed) {
      fresh.close()
      throw new NavigateError("unavailable", "the client is closed")
    }

    return fresh
  }

  return {
    async navigate(request, options = {}) {
      if (closed) throw new NavigateError("unavailable", "the client is closed")

      return (await (await connection()).call(request, options)) as never
    },
    close() {
      closed = true
      void open?.then((current) => current.close()).catch(() => undefined)
    }
  }
}
