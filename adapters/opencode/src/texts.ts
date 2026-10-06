import type { Plugin } from "@opencode/plugin/effect"
import { Context, type Duration, Effect, Schedule, Schema, type Scope } from "effect"
import shipped from "../../../skills/config/hosts/opencode.json" with { type: "json" }
import { Daemon } from "./daemon.js"

/** How often the daemon's texts are re-read; it reloads your file when it changes. */
const REFRESH: Duration.Input = "5 seconds"

/**
 * The wording this adapter shows the agent, from `skills/config/hosts/opencode.json`.
 * Code decides when each text is shown; the words are data.
 */
const HostTextsSchema = Schema.Struct({
  ask_chauffeur: Schema.Struct({
    description: Schema.String,
    need: Schema.String,
    nothing_found: Schema.String,
    nothing_found_skill_tool: Schema.String,
    revealed: Schema.String,
    failed: Schema.String,
  }),
  todowrite: Schema.Struct({
    description: Schema.String,
    todos: Schema.String,
    content: Schema.String,
    status: Schema.String,
    too_many: Schema.String,
    empty: Schema.String,
    heading: Schema.String,
  }),
  subagent: Schema.Struct({ guidance: Schema.String }),
  rulebook: Schema.Struct({
    description: Schema.String,
    usage: Schema.String,
    usage_with_args: Schema.String,
    too_long: Schema.String,
    failed: Schema.String,
  }),
  permission: Schema.Struct({ failed: Schema.String }),
  labels: Schema.Struct({ context: Schema.String, exposure: Schema.String }),
})

export type HostTexts = typeof HostTextsSchema.Type

/** The shipped texts, bundled so the agent sees wording even while the daemon is down. */
export const SHIPPED_TEXTS: HostTexts = Schema.decodeUnknownSync(HostTextsSchema)(shipped)

/**
 * `template` with each `{name}` replaced by its value, in one pass, so a value
 * holding braces is never expanded again. Unknown placeholders stay as written.
 */
export function fill(template: string, values: Readonly<Record<string, string>>): string {
  return template.replace(/\{([a-z0-9_]+)\}/g, (match, name: string) => values[name] ?? match)
}

/** The texts as they stand now, and a counter that moves when they change. */
export type TextsSource = {
  readonly current: () => HostTexts
  readonly version: () => number
}

export class Texts extends Context.Service<Texts, TextsSource>()("chauffeur/Texts") {}

/** Fixed texts, such as the shipped ones. */
export function fixedTexts(texts: HostTexts = SHIPPED_TEXTS): TextsSource {
  return { current: () => texts, version: () => 0 }
}

/**
 * The daemon's texts (shipped, overlaid by your file), re-read every few
 * seconds. Until the daemon answers, and whenever it sends texts this adapter
 * cannot read, the last good texts stand.
 */
export const followDaemonTexts: Effect.Effect<TextsSource, never, Daemon | Scope.Scope> = Effect.gen(function* () {
  const daemon = yield* Daemon
  let texts = SHIPPED_TEXTS
  let seen = JSON.stringify(texts)
  let version = 0

  const refresh = daemon.texts("opencode").pipe(
    Effect.flatMap(Schema.decodeUnknownEffect(HostTextsSchema)),
    Effect.map((fetched) => {
      const json = JSON.stringify(fetched)

      if (json === seen) return

      texts = fetched
      seen = json
      version += 1
    }),
    Effect.catch((error) => Effect.logError("chauffeur: texts unavailable; keeping the last good ones", error)),
  )

  yield* refresh
  yield* refresh.pipe(Effect.repeat(Schedule.spaced(REFRESH)), Effect.forkScoped)

  return { current: () => texts, version: () => version }
})

/** A set of registered tools or commands, removed by disposing it. */
export type Registration = Effect.Success<ReturnType<Plugin.Context["tool"]["transform"]>>

/**
 * Register with the current texts, and again whenever they change, disposing
 * the previous registration, so edited wording reaches the agent without a
 * restart.
 */
export function withTexts<R>(
  register: (texts: HostTexts) => Effect.Effect<Registration, never, R | Scope.Scope>,
  refresh: Duration.Input = REFRESH,
): Effect.Effect<void, never, R | Texts | Scope.Scope> {
  return Effect.gen(function* () {
    const source = yield* Texts
    let seen = source.version()
    let registration = yield* register(source.current())

    const follow = Effect.gen(function* () {
      if (source.version() === seen) return

      seen = source.version()
      yield* registration.dispose
      registration = yield* register(source.current())
    })

    yield* follow.pipe(Effect.repeat(Schedule.spaced(refresh)), Effect.forkScoped)
  })
}
