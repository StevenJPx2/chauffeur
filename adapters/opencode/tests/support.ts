import type { Plugin } from "@opencode/plugin/effect"
import { Effect, Exit, Queue, Scope, Stream } from "effect"
import { Daemon, type DaemonClient } from "../src/daemon.js"
import { Host, type SessionID } from "../src/host.js"

type Callback = (event: never) => Effect.Effect<void>

type Member = ((...args: never[]) => Effect.Effect<unknown, unknown> | Stream.Stream<unknown, unknown>) | FakeDomain | string

type FakeDomain = { readonly [member: string]: Member }

/** The host members one test supplies; any member it omits is absent. */
export type FakeHost = { readonly [Domain in keyof Plugin.Context]?: FakeDomain }

/** A test double for the host: only the members a capability under test reads. */
export function fakeHost(members: FakeHost): Plugin.Context {
  // oxlint-disable-next-line anti-slop/no-chained-type-assertions, anti-slop/require-safety-comment-for-type-assertion -- a test double supplies only the members its test reads; a missing one fails that test.
  return members as unknown as Plugin.Context
}

/** Hook registrations, run as the host would run them. */
export class Hooks {
  private readonly callbacks = new Map<string, Callback>()

  readonly register = (name: string, callback: Callback) => Effect.sync(() => {
    this.callbacks.set(name, callback)

    return { dispose: Effect.void }
  })

  emit<Event>(name: string, event: Event): Promise<void> {
    const callback = this.callbacks.get(name)

    // SAFETY: each test emits the host's event for the hook it names, with the fields its capability reads.
    return callback ? Effect.runPromise(callback(event as never)) : Promise.resolve()
  }
}

type HostEvent = { readonly type: string; readonly data: { readonly sessionID: string } }

/** A host event stream the test publishes to. */
export function eventStream() {
  const queue = Effect.runSync(Queue.unbounded<HostEvent>())

  return {
    subscribe: () => Stream.fromQueue(queue),
    publish: (event: HostEvent) => Effect.runPromise(Queue.offer(queue, event)),
  }
}

/** Run a capability's install in its own scope, as the plugin scope would. */
export async function install<A>(
  program: Effect.Effect<A, never, Host | Daemon | Scope.Scope>,
  host: Plugin.Context,
  daemon: DaemonClient,
): Promise<{ readonly value: A; readonly close: () => Promise<void> }> {
  const scope = await Effect.runPromise(Scope.make())

  const value = await Effect.runPromise(program.pipe(
    Effect.provideService(Host, host),
    Effect.provideService(Daemon, daemon),
    Scope.provide(scope),
  ))

  return { value, close: () => Effect.runPromise(Scope.close(scope, Exit.void)) }
}

/** Let forked fibers process what the test just published. */
export function settle(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 10))
}

export const noDaemon: DaemonClient = { signal: () => Effect.die("unexpected daemon signal") }

/** A host session ID for a session the test names. */
export function sessionID(value: string): SessionID {
  // SAFETY: host session IDs are opaque strings; the host never validates their shape.
  return value as SessionID
}
