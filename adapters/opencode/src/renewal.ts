import type { SessionCompaction } from "@opencode/plugin/effect/session"
import { Data, DateTime, Duration, Effect, Fiber, Predicate, Schema, type Scope, Stream } from "effect"
import { Daemon } from "./daemon.js"
import { currentContext, type HistoryMessage, Host, type SessionID } from "./host.js"
import { available, hostModel, modelKey, ref } from "./model-router.js"
import { signal, type ModelRef } from "./protocol.js"
import { fill, type HostTexts, Texts } from "./texts.js"
import type { WarmingControl } from "./warming.js"

const MAX_SESSIONS = 256

/** Admission waits this long for the engine's renewal decision, then proceeds as it is. */
const DECISION_TIMEOUT: Duration.Input = "2 seconds"

/** How long the cheap model may take to write the summary before OpenCode compacts on the session's model. */
const SUMMARY_TIMEOUT: Duration.Input = "3 minutes"

/**
 * A queued compaction OpenCode has not started within this is forgotten, so
 * a later compaction of OpenCode's own, on a cache still warm, keeps the
 * session's model. A queued one starts within seconds.
 */
const PENDING_MS = 2 * 60 * 1_000

/** The window assumed for a summary model whose window the host does not report, as OpenCode assumes. */
const UNKNOWN_WINDOW = 200_000

/** Room kept in the summary model's window for its reply. */
const REPLY_TOKENS = 16_000

/** Characters per token when sizing the transcript; low, so a dense transcript still fits. */
const CHARS_PER_TOKEN = 3

/** How much of one tool's output the transcript keeps, as OpenCode's own transcript does. */
const TOOL_OUTPUT_CHARS = 1_250

const METADATA_KEY = "chauffeur.compaction"

/** The temporary session a summary is written in. */
const SUMMARY_TITLE = "Chauffeur context summary"

/**
 * How long a provider whose summary request failed is passed over, so the
 * next renewal asks another provider. Short: a usage limit or a spent free
 * allowance usually clears within the hour.
 */
const PASS_OVER_MS = 60 * 60 * 1_000

const PASS_OVER_KEY = "renewal.passed-over"

/** A summary the model wrote without the template's headings. */
class UnfilledTemplate extends Data.TaggedError("UnfilledTemplate")<{ readonly model: string }> {}

const Deadlines = Schema.Record(Schema.String, Schema.Number)

type HookMessage = SessionCompaction["messages"][number]

type HostModel = { readonly providerID: string; readonly id: string; readonly variant?: string | undefined }

type ListedModel = HostModel & CacheSettings & { readonly enabled: boolean; readonly status: string; readonly limit: { readonly context: number; readonly input?: number | undefined } }

/** Where a listed model carries the user's prompt-cache settings. */
export type CacheSettings = { readonly settings?: unknown; readonly variants?: ReadonlyArray<{ readonly id: string; readonly settings?: unknown }> }

/** A compaction Chauffeur queued, and the model that writes its summary. */
type Pending = { readonly model: ModelRef; readonly window: number; readonly at: number }

export type RenewalControl = {
  /**
   * Before a prompt is admitted: when the engine says the session's context
   * should be renewed, queue a compaction ahead of the prompt, to be written
   * by the engine's cheaper model. True when one was queued.
   */
  readonly renew: (sessionID: SessionID) => Effect.Effect<boolean>
}

export const NO_RENEWAL: RenewalControl = { renew: () => Effect.succeed(false) }

/** The context's size and idle time, from its latest reply the provider measured. */
export type ContextFacts = { readonly model: HostModel; readonly tokens: number; readonly idleSeconds: number }

/**
 * Renews a long context that has gone cold. The engine judges whether to;
 * this queues OpenCode's compaction ahead of the prompt and has the engine's
 * cheaper model write the summary in OpenCode's compaction hook. The
 * session's own model never changes, so the prompt runs on it as before.
 * If the cheaper model fails, OpenCode compacts on the session's model.
 */
export const installRenewal = (warming: WarmingControl = { lastWarmed: () => undefined, expiring: () => undefined }): Effect.Effect<RenewalControl, never, Host | Daemon | Texts | Scope.Scope> => Effect.gen(function* () {
  const host = yield* Host
  const daemon = yield* Daemon
  const texts = yield* Texts
  const pending = new Map<string, Pending>()

  const stored = yield* host.storage.get(PASS_OVER_KEY).pipe(
    Effect.flatMap(Schema.decodeUnknownEffect(Deadlines)),
    Effect.orElseSucceed(() => ({})),
  )

  /** Providers whose summary request failed, until when they are passed over. */
  const passedOver = new Map(Object.entries(stored))

  const passOver = (provider: string) => {
    passedOver.set(provider, Date.now() + PASS_OVER_MS)

    return host.storage.set(PASS_OVER_KEY, Object.fromEntries(passedOver)).pipe(Effect.ignore)
  }

  const reachable = (provider: string) => (passedOver.get(provider) ?? 0) <= Date.now()

  yield* host.session.hook("compaction", (event) => {
    const sessionID = String(event.sessionID)
    const job = pending.get(sessionID)

    pending.delete(sessionID)

    if (!job || Date.now() - job.at > PENDING_MS) return Effect.void

    return summarize(event.sessionID, job.model, summaryRequest(event.messages, job.window, texts.current())).pipe(
      Effect.timeout(SUMMARY_TIMEOUT),
      Effect.flatMap((text) => filled(text, texts.current())
        ? Effect.sync(() => { event.result = { summary: text.trim(), metadata: { [METADATA_KEY]: modelKey(job.model) } } })
        : Effect.fail(new UnfilledTemplate({ model: modelKey(job.model) }))),
      Effect.catch((error) => Effect.logWarning("chauffeur: the cheaper summary failed; OpenCode compacts on the session's model", error).pipe(
        // A model that answered, if poorly, can still be asked; one that failed is passed over for a while.
        Effect.andThen(Predicate.isTagged(error, "UnfilledTemplate") ? Effect.void : passOver(job.model.provider)),
      )),
    )
  })

  /**
   * `prompt` answered by `model` in a temporary child session of `parentID`,
   * removed afterwards. A session request goes through the provider's
   * session sign-in, such as a subscription plugin's, which a bare
   * generation does not reach.
   */
  const summarize = (parentID: SessionID, model: ModelRef, prompt: string) => Effect.acquireUseRelease(
    host.session.create({ parentID, title: SUMMARY_TITLE, model: hostModel(model) }),
    (child) => host.session.generate({ sessionID: child.id, prompt }).pipe(Effect.map(({ text }) => text)),
    (child) => host.session.remove({ sessionID: child.id }).pipe(Effect.ignore),
  )

  /**
   * Asks the engine whether a context resumed after `idleSeconds` should be
   * renewed first; on a yes, queues OpenCode's compaction ahead of the new
   * input, to be written by the engine's cheaper model. True when one was queued.
   */
  const judge = (sessionID: SessionID, model: HostModel, tokens: number, idleSeconds: number) => Effect.gen(function* () {
    const models: ReadonlyArray<ListedModel> = (yield* host.model.list()).data

    const effects = yield* daemon.signal(signal(String(sessionID), {
      type: "context_resumed",
      model: ref(model),
      context_tokens: tokens,
      context_window: windowOf(models, ref(model)) ?? 0,
      idle_seconds: idleSeconds,
      cache_seconds: cacheSeconds(models.find((entry) => entry.providerID === model.providerID && entry.id === model.id), model.variant),
      available: available(models).map((entry) => (reachable(entry.model.provider) ? entry : { ...entry, usable: false })),
    }), DECISION_TIMEOUT)

    const compact = effects.find((effect) => effect.type === "compact" && effect.agent_id === String(sessionID))

    if (compact?.type !== "compact") return false

    if (!pending.has(String(sessionID)) && pending.size >= MAX_SESSIONS) pending.clear()

    pending.set(String(sessionID), { model: compact.model, window: windowOf(models, compact.model) ?? UNKNOWN_WINDOW, at: Date.now() })

    yield* host.session.compact({ sessionID }).pipe(Effect.tapError(() => Effect.sync(() => pending.delete(String(sessionID)))))

    return true
  }).pipe(
    // Renewal fails open: the input runs on the context as it is.
    Effect.catch((error) => Effect.logError("chauffeur: context renewal unavailable", error).pipe(Effect.as(false))),
  )

  /** Judges the context as it is now, from its latest measured reply. */
  const renew = (sessionID: SessionID) => Effect.gen(function* () {
    const facts = contextFacts(yield* host.session.context({ sessionID }), Date.now(), warming.lastWarmed(String(sessionID)))

    if (!facts) return false

    return yield* judge(sessionID, (yield* host.session.get({ sessionID })).model ?? facts.model, facts.tokens, facts.idleSeconds)
  }).pipe(
    // Renewal fails open: the input runs on the context as it is.
    Effect.catch((error) => Effect.logError("chauffeur: context renewal unavailable", error).pipe(Effect.as(false))),
  )

  yield* watchExpiry(renew, warming)

  return { renew }
})


/**
 * Renews an idle session once its prompt cache has expired, when the engine
 * said at its turn's end that it would be worth it. Input other than a
 * prompt, such as an integration's synthetic message or a finished shell's
 * notice, reaches the model before any hook could compact the context, so
 * the context is renewed while nothing runs: whatever comes next finds it
 * renewed instead of re-reading it cold. The session's next execution
 * cancels its timer; `renew` asks the engine again with the idle time then.
 */
function watchExpiry(renew: (sessionID: SessionID) => Effect.Effect<boolean>, warming: WarmingControl): Effect.Effect<void, never, Host | Scope.Scope> {
  return Effect.gen(function* () {
    const host = yield* Host
    const scope = yield* Effect.scope
    const timers = new Map<string, Fiber.Fiber<void>>()

    const cancel = (id: string) => {
      const timer = timers.get(id)

      timers.delete(id)

      return timer ? Fiber.interrupt(timer).pipe(Effect.forkIn(scope), Effect.asVoid) : Effect.void
    }

    warming.expiring((sessionID, afterSeconds) => Effect.gen(function* () {
      const id = String(sessionID)

      yield* cancel(id)

      if (timers.size >= MAX_SESSIONS) yield* Effect.forEach([...timers.keys()], cancel, { discard: true })

      timers.set(id, yield* Effect.sleep(Duration.seconds(afterSeconds)).pipe(
        Effect.andThen(Effect.sync(() => { timers.delete(id) })),
        Effect.andThen(renew(sessionID)),
        Effect.asVoid,
        Effect.forkIn(scope),
      ))
    }))

    yield* host.event.subscribe().pipe(
      Stream.runForEach((event) => (event.type === "session.execution.started" ? cancel(String(event.data.sessionID)) : Effect.void)),
      Effect.catch((error) => Effect.logError("chauffeur: renewal events ended", error)),
      Effect.forkScoped,
    )
  })
}

/**
 * The context's size at its latest measured reply, as OpenCode measures it,
 * and the seconds since the provider last read or wrote its prompt cache:
 * since that reply ended, or since a later warming request at `warmed`.
 * Undefined for a context without a measured reply.
 */
export function contextFacts(messages: ReadonlyArray<HistoryMessage>, now: number, warmed = 0): ContextFacts | undefined {
  const reply = currentContext(messages).findLast((message) =>
    message.type === "assistant" && !message.error && message.tokens !== undefined
    && message.tokens.input + message.tokens.cache.read + message.tokens.cache.write > 0)

  if (reply?.type !== "assistant" || !reply.tokens) return undefined

  const { tokens } = reply
  const ended = DateTime.toEpochMillis(reply.time.completed ?? reply.time.created)

  return {
    model: reply.model,
    tokens: tokens.input + tokens.cache.read + tokens.cache.write + tokens.output + tokens.reasoning,
    idleSeconds: Math.max(0, Math.floor((now - Math.max(ended, warmed)) / 1_000)),
  }
}

/** A prompt-cache lifetime as OpenCode's `cache.ttl` writes it, such as "5m" or "1h". */
const TTL = /^(\d+)(ms|s|m|h|d)$/

const UNIT_SECONDS = new Map([["ms", 0.001], ["s", 1], ["m", 60], ["h", 3_600], ["d", 86_400]])

/** Settings that set a prompt-cache lifetime. */
const CacheTTL = Schema.Struct({ cache: Schema.Struct({ ttl: Schema.String }) })

const setsTTL = Schema.is(CacheTTL)

/**
 * The prompt-cache lifetime the user set for a listed model, in whole
 * seconds: its `variant`'s `cache.ttl`, else the model's, which OpenCode
 * resolves from the provider's. Undefined when neither is set.
 */
export function cacheSeconds(listed: CacheSettings | undefined, variant?: string | undefined): number | undefined {
  const own = listed?.variants?.find((entry) => entry.id === variant)?.settings
  const settings = [own, listed?.settings].find(setsTTL)
  const match = settings ? TTL.exec(settings.cache.ttl) : null
  const seconds = match ? Math.floor(Number(match[1]) * (UNIT_SECONDS.get(match[2] ?? "") ?? 0)) : 0

  return seconds > 0 ? seconds : undefined
}

/** The input window the host reports for `model`, if it does. */
export function windowOf(models: ReadonlyArray<ListedModel>, model: ModelRef): number | undefined {
  const listed = models.find((entry) => entry.providerID === model.provider && entry.id === model.model)
  const window = listed ? listed.limit.input || listed.limit.context : 0

  return window > 0 ? window : undefined
}

/**
 * The summary request: the conversation as text, within the summary model's
 * window, then the instructions. An earlier summary leads and is always kept;
 * whole exchanges are left out oldest first until the rest fit.
 */
export function summaryRequest(messages: ReadonlyArray<HookMessage>, window: number, texts: HostTexts): string {
  const wording = texts.compaction
  const update = messages.some(isCheckpoint)
  const instructions = [update ? wording.update : wording.first, wording.template].join("\n\n")
  const room = (window - Math.max(Math.floor(window * 0.1), REPLY_TOKENS)) * CHARS_PER_TOKEN - instructions.length - wording.request.length
  const lead = messages.findIndex(isCheckpoint) + 1

  const exchanges = messages.slice(lead).reduce<string[]>((groups, message) => {
    const text = flatten(message)

    if (!text) return groups

    if (message.role === "user" || groups.length === 0) groups.push(text)
    else groups[groups.length - 1] += `\n${text}`

    return groups
  }, [])

  const leading = messages.slice(0, lead).flatMap((message) => flatten(message) || []).join("\n")

  let left = room - leading.length
  let start = exchanges.length

  for (const exchange of exchanges.toReversed()) {
    if (left < exchange.length + 2) break

    left -= exchange.length + 2
    start -= 1
  }

  const omitted = start > 0 ? fill(wording.omitted, { count: String(start) }) : ""
  const transcript = [leading, omitted, ...exchanges.slice(start)].filter(Boolean).join("\n\n")

  return fill(wording.request, { transcript, instructions })
}

/** Whether the reply fills the template: it uses at least one of its headings. */
function filled(text: string, texts: HostTexts): boolean {
  const headings = texts.compaction.template.split("\n").map((line) => line.trim()).filter((line) => line.startsWith("##"))

  return text.split("\n").some((line) => headings.includes(line.trim()))
}

/** An earlier compaction: a native item, or OpenCode's checkpoint message. */
function isCheckpoint(message: HookMessage): boolean {
  return message.content.some((part) => part.type === "compaction" || (part.type === "text" && part.text.includes("<conversation-checkpoint>")))
}

/** One message as transcript lines; reasoning, instruction updates and media bodies are left out. */
function flatten(message: HookMessage): string {
  if (message.role === "system") return ""

  const speaker = message.role === "user" ? "User" : "Assistant"

  return message.content.flatMap((part) => line(part, speaker) || []).join("\n")
}

type Part = HookMessage["content"][number]

/** One part as a transcript line; empty for what the transcript leaves out. */
function line(part: Part, speaker: string): string {
  switch (part.type) {
    case "text":
      return labelled(speaker, part.text)
    case "compaction":
      return labelled("Earlier summary", part.text)
    case "media":
      return `[${part.media.mediaType} omitted]`
    case "tool-call":
      return labelled("Assistant tool call", `${part.name}(${JSON.stringify(part.input)})`)
    case "tool-result":
      return resultLine(part.result)
    default:
      return ""
  }
}

/** `[label]: text`, or nothing for empty text. */
function labelled(label: string, text: string | null | undefined): string {
  return text ? `[${label}]: ${text}` : ""
}

type ToolResult = Extract<Part, { readonly type: "tool-result" }>["result"]

function resultLine(result: ToolResult): string {
  return labelled(result.type === "error" ? "Tool error" : "Tool result", clipOutput(resultText(result)))
}

const isText = Schema.is(Schema.String)

/** A tool result as text: its text content, else its value as JSON. */
function resultText(result: ToolResult): string {
  if (result.type === "content") return result.value.map((item) => (item.type === "text" ? item.text : `[Attached ${item.mime}]`)).join("\n")

  return isText(result.value) ? result.value : JSON.stringify(result.value) ?? ""
}

function clipOutput(text: string): string {
  const points = [...text]

  return points.length <= TOOL_OUTPUT_CHARS ? text : `${points.slice(0, TOOL_OUTPUT_CHARS).join("")}\n[truncated]`
}
