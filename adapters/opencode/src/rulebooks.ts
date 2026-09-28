import { Skill } from "@opencode/plugin/effect"
import { Effect, type Scope } from "effect"
import { Daemon } from "./daemon.js"
import { Host, type SessionID } from "./host.js"
import { signal, type RulebookCommand, type RulebookEntry } from "./protocol.js"
import { deliverContext } from "./skills.js"
import { userText } from "./text.js"

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
 * One slash command per rulebook the daemon offers in this plugin's
 * location, such as `/goal` everywhere and `/ticket` only where it is in
 * scope; the host keeps commands per location. The engine keeps each
 * session's books and answers every command with the context to deliver, so
 * this adapter knows nothing of any rulebook.
 */
export const installRulebooks: Effect.Effect<void, never, Host | Daemon | Scope.Scope> = Effect.gen(function* () {
  const host = yield* Host
  const daemon = yield* Daemon

  const books = yield* daemon.rulebooks(String(host.location.directory)).pipe(
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
