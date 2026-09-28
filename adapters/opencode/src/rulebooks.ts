import { Effect, type Scope } from "effect"
import { Daemon } from "./daemon.js"
import { Host, type SessionID } from "./host.js"
import { signal, TEXT_CODE_POINTS, type RulebookCommand, type RulebookEntry } from "./protocol.js"
import { deliverContext } from "./skills.js"
import { clip } from "./text.js"

const COMMANDS: ReadonlyArray<RulebookCommand> = ["pause", "resume", "clear", "status"]

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

/**
 * One slash command per rulebook the daemon offers, such as `/goal`. The
 * engine keeps each session's rulebook and answers every command with the
 * context to deliver, so this adapter knows nothing of any rulebook.
 */
export const installRulebooks: Effect.Effect<void, never, Host | Daemon | Scope.Scope> = Effect.gen(function* () {
  const host = yield* Host
  const daemon = yield* Daemon

  const books = yield* daemon.rulebooks().pipe(
    Effect.catch((error) => Effect.logError("chauffeur: rulebooks unavailable", error).pipe(Effect.as([]))),
  )

  if (books.length === 0) return

  yield* host.command.transform((editor) => {
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

function description(book: RulebookEntry): string {
  const usage = book.args_required ? `/${book.id} <what>` : `/${book.id}`

  return `${book.description} ${usage}, or pause | resume | clear.`
}

function run(sessionID: SessionID, rulebook: string, text: string): Effect.Effect<void, never, Host | Daemon> {
  return Effect.gen(function* () {
    const daemon = yield* Daemon
    const { command, args } = parseRulebookInput(text)

    const effects = yield* daemon.signal(signal(String(sessionID), {
      type: "rulebook",
      command,
      rulebook,
      args: clip(args, TEXT_CODE_POINTS),
    }))

    const notices = effects.flatMap((effect) =>
      effect.type === "context" && effect.agent_id === String(sessionID) ? [effect] : [])

    yield* Effect.forEach(notices, (effect) => deliverContext(sessionID, effect), { discard: true })
  }).pipe(
    Effect.catch((error) => Effect.logError(`chauffeur: /${rulebook} failed`, error)),
  )
}
