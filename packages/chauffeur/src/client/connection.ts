import { createConnection, type Socket } from "node:net"
import {
  NavigateError,
  PROTOCOL,
  rejected,
  type NavigateRequest,
  type NavigationEvent
} from "./wire.ts"

/** The longest line read from the daemon; past it the connection is dropped. */
const MAX_LINE = 4 * 1024 * 1024

/** How much longer than the request's deadline the daemon is waited for. */
const MARGIN_MS = 2_000

/** The daemon's deadline for a request that names none. */
const DEFAULT_DEADLINE_MS = 3_000

/** How one call is waited for. */
export type CallOptions = {
  readonly signal?: AbortSignal
  /** Each event of the navigation, as it runs. */
  readonly onEvent?: (event: NavigationEvent) => void
}

type Pending = {
  readonly resolve: (result: unknown) => void
  readonly reject: (error: unknown) => void
  readonly onEvent: ((event: NavigationEvent) => void) | undefined
}

/**
 * One connection to the daemon, carrying any number of calls at once, each
 * by its own ID. A call settles on its reply, the daemon closing, its
 * signal aborting, or the request's deadline and two seconds passing.
 */
export class Connection {
  readonly #socket: Socket
  readonly #pending = new Map<number, Pending>()
  #received = ""
  #next = 1
  #closed = false

  private constructor(socket: Socket) {
    this.#socket = socket
    socket.setEncoding("utf8")
    socket.on("data", (data: string) => this.#read(data))
    socket.on("close", () => this.#end("the daemon closed the connection"))
    socket.on("error", () => socket.destroy())
  }

  /** Connects to the daemon on `path`; rejects as `unavailable` when none answers. */
  static open(path: string): Promise<Connection> {
    return new Promise((resolve, reject) => {
      const socket = createConnection(path)
      const failed = (error: Error) => {
        reject(
          new NavigateError(
            "unavailable",
            `no daemon answers on ${path}: ${error.message}`
          )
        )
      }

      socket.once("error", failed)
      socket.once("connect", () => {
        socket.off("error", failed)
        resolve(new Connection(socket))
      })
    })
  }

  /** Whether the connection has closed; it carries no more calls. */
  get closed(): boolean {
    return this.#closed
  }

  /** Sends `request` and settles with its result. */
  call(request: NavigateRequest, options: CallOptions = {}): Promise<unknown> {
    const { signal, onEvent } = options

    if (this.#closed) {
      return Promise.reject(
        new NavigateError("unavailable", "the connection is closed")
      )
    }

    if (signal?.aborted) return Promise.reject(signal.reason)

    const id = this.#next++
    const wait = (request.deadline_ms ?? DEFAULT_DEADLINE_MS) + MARGIN_MS
    const params = { protocol: PROTOCOL, ...request }

    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.#cancel(id)
        settle.reject(
          new NavigateError("unavailable", `no answer within ${wait} ms`)
        )
      }, wait)
      const aborted = () => {
        this.#cancel(id)
        settle.reject(signal?.reason)
      }
      const done = () => {
        clearTimeout(timer)
        signal?.removeEventListener("abort", aborted)
        this.#pending.delete(id)
      }
      const settle: Pending = {
        resolve: (result) => (done(), resolve(result)),
        reject: (error) => (done(), reject(error)),
        onEvent
      }

      this.#pending.set(id, settle)
      signal?.addEventListener("abort", aborted, { once: true })
      this.#write({ jsonrpc: "2.0", id, method: "chauffeur.navigate", params })
    })
  }

  /** Closes the connection; the daemon cancels every call it carried. */
  close(): void {
    this.#socket.end()
    this.#socket.destroy()
    this.#end("the connection was closed")
  }

  #cancel(id: number): void {
    if (this.#closed || !this.#pending.has(id)) return

    this.#write({ jsonrpc: "2.0", method: "$/cancelRequest", params: { id } })
  }

  #write(message: unknown): void {
    this.#socket.write(`${JSON.stringify(message)}\n`)
  }

  #read(data: string): void {
    const received = this.#received + data
    let start = 0
    let end = received.indexOf("\n")

    while (end !== -1) {
      this.#message(received.slice(start, end))
      start = end + 1
      end = received.indexOf("\n", start)
    }

    this.#received = received.slice(start)

    if (this.#received.length > MAX_LINE) this.#socket.destroy()
  }

  #message(line: string): void {
    let message: {
      id?: unknown
      method?: unknown
      params?: { id?: unknown } & Record<string, unknown>
      result?: unknown
      error?: Parameters<typeof rejected>[0]
    }

    try {
      message = JSON.parse(line)
    } catch {
      return
    }

    if (message.method === "chauffeur.event") {
      const { id, ...event } = message.params ?? {}
      const pending = typeof id === "number" ? this.#pending.get(id) : undefined

      // A listener that throws ends its own call, never the connection's others.
      try {
        pending?.onEvent?.(event as NavigationEvent)
      } catch (error) {
        if (typeof id === "number") this.#cancel(id)
        pending?.reject(error)
      }

      return
    }

    const pending =
      typeof message.id === "number" ? this.#pending.get(message.id) : undefined

    if (!pending) return

    if (message.error) pending.reject(rejected(message.error))
    else pending.resolve(message.result)
  }

  #end(why: string): void {
    this.#closed = true

    for (const pending of [...this.#pending.values()]) {
      pending.reject(new NavigateError("unavailable", why))
    }
  }
}
