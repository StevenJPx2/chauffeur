import { type Plugin, Skill } from "@opencode/plugin/effect"
import { type Duration, Effect, Schedule, type Scope } from "effect"
import { Daemon } from "./daemon.js"
import { Host, type SessionID } from "./host.js"
import { signal, type RulebookCommand, type RulebookEntry } from "./protocol.js"
import { deliverContext } from "./skills.js"
import { userText } from "./text.js"

const COMMANDS: ReadonlyArray<RulebookCommand> = ["pause", "resume", "clear", "status"]

/** A set of registered commands, removed by disposing it. */
type Registration = Effect.Success<ReturnType<Plugin.Context["command"]["transform"]>>

/** What the user asked of a rulebook, and the text it starts with. */
type RulebookInput = { readonly command: RulebookCommand; readonly args: string }

/**
 * `/goal pause` pauses; bare `/goal` asks for status; any other text starts
 * the rulebook with that text as its arguments.
 */
export function parseRulebookInput(text: string): RulebookInput {
  const args = text.trim()
  const word = args.toLowerCase()

  if (args === "") return { command: "status", args: "" }

  const command = COMMANDS.find((candidate) => candidate === word)

  return command ? { command, args: "" } : { command: "start", args }
}

/** How often the offered rulebooks are re-read; the daemon reloads them from disk. */
const REFRESH: Duration.Input = "5 seconds"

/**
 * One slash command per rulebook the daemon offers in this plugin's
 * location, such as `/goal` everywhere and `/ticket` only where it is in
 * scope; the host keeps commands per location. The engine keeps each
 * session's books and answers every command with the context to deliver, so
 * this adapter knows nothing of any rulebook.
 *
 * The offer is re-read every few seconds, and the commands are registered
 * again whenever it changes, so a rulebook added, edited or removed on disk
 * shows up without restarting OpenCode.
 */
export const installRulebooks: Effect.Effect<void, never, Host | Daemon | Scope.Scope> = Effect.suspend(() => offerRulebooks(REFRESH))

/** The offer the registered commands came from, as JSON, and their registration. */
interface Installed {
  readonly offer: string
  readonly registration: Registration | undefined
}

/** [`installRulebooks`], re-reading the offer every `refresh`. */
export function offerRulebooks(refresh: Duration.Input): Effect.Effect<void, never, Host | Daemon | Scope.Scope> {
  return Effect.gen(function* () {
    const host = yield* Host
    const daemon = yield* Daemon
    const workspace = String(host.location.directory)
    let installed: Installed = { offer: "[]", registration: undefined }

    const sync = Effect.gen(function* () {
      // An unreachable daemon keeps the commands already registered.
      const books = yield* daemon.rulebooks(workspace).pipe(
        Effect.catch((error) => Effect.logError("chauffeur: rulebooks unavailable", error).pipe(Effect.as(undefined))),
      )

      const offer = JSON.stringify(books)

      if (books === undefined || offer === installed.offer) return

      if (installed.registration) yield* installed.registration.dispose

      installed = { offer, registration: books.length === 0 ? undefined : yield* register(books) }

      // Registry changes are lazy: the next read rebuilds and publishes them.
      // Materialize now so an idle TUI sees the new commands without a prompt.
      yield* host.command.list().pipe(
        Effect.catch((error) => Effect.logError("chauffeur: command refresh unavailable", error)),
      )
    })

    yield* sync
    yield* sync.pipe(Effect.repeat(Schedule.spaced(refresh)), Effect.forkScoped)
  })
}

/** One command per rulebook, registered together so they are removed together. */
function register(books: ReadonlyArray<RulebookEntry>): Effect.Effect<Registration, never, Host | Daemon | Scope.Scope> {
  return Effect.gen(function* () {
    const host = yield* Host
    const daemon = yield* Daemon

    return yield* host.command.transform((editor) => {
      for (const book of books) {
        editor.add({
          name: book.id,
          description: description(book),
          execute: ({ sessionID, prompt }) =>
            run(sessionID, book.id, prompt.text).pipe(
              Effect.provideService(Host, host),
              Effect.provideService(Daemon, daemon),
            ),
        })
      }
    })
  })
}

function description(book: RulebookEntry): string {
  const usage = book.args_required ? `/${book.id} <what>` : `/${book.id}`

  return `${book.description} ${usage}, or pause | resume | clear.`
}

/**
 * The user's arguments reach the engine whole. Its answer that wakes the
 * agent (a start or resume, carrying the goal) becomes a visible prompt;
 * every other answer is a Chauffeur note.
 */
function run(sessionID: SessionID, rulebook: string, text: string): Effect.Effect<void, never, Host | Daemon> {
  return Effect.gen(function* () {
    const host = yield* Host
    const daemon = yield* Daemon
    const { command, args } = parseRulebookInput(text)

    if (userText(args) !== args) {
      yield* note(sessionID, rulebook, `/${rulebook} text is ${Math.ceil(Buffer.byteLength(args, "utf8") / 1_024)} KiB; the limit is 64 KiB. Nothing was started: shorten it and try again.`)

      return
    }

    const workspace = String(host.location.directory)
    const effects = yield* daemon.signal(signal(String(sessionID), { type: "rulebook", command, rulebook, args, workspace }))

    const answers = effects.flatMap((effect) =>
      effect.type === "context" && effect.agent_id === String(sessionID) ? [effect] : [])

    // A start or resume shows as a prompt; OpenCode attaches its skills.
    yield* Effect.forEach(answers, (effect) =>
      effect.delivery === "resume" && effect.text
        ? host.session.prompt({
          sessionID,
          text: effect.text,
          skills: effect.skills.map((id) => ({ id: Skill.ID.make(id) })),
          metadata: { [RULEBOOK_METADATA_KEY]: rulebook },
        }).pipe(Effect.asVoid)
        : deliverContext(sessionID, effect), { discard: true })
  }).pipe(
    Effect.catch((error) => note(sessionID, rulebook, `/${rulebook} failed: ${String(error)}`).pipe(
      Effect.catch(() => Effect.logError(`chauffeur: /${rulebook} failed`, error)),
    )),
  )
}

/** Visible rulebook prompts record the rulebook here. */
const RULEBOOK_METADATA_KEY = "chauffeur.rulebook"

function note(sessionID: SessionID, rulebook: string, text: string): Effect.Effect<void, unknown, Host> {
  return deliverContext(sessionID, { type: "context", agent_id: String(sessionID), delivery: "wait", label: `/${rulebook}`, skills: [], text })
}
