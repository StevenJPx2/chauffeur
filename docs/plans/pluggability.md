# Chauffeur v2 implementation plan

Status: design for review. The starting workspace is clean apart from its
toolchain, license, plan/audit documents and the Vite/WebAssembly playground.
The [v1 architecture audit](../audits/architecture.md) records the source
findings and checks that inform this plan.

## Goal

Chauffeur is a judgment engine with plugins around it.

- **The engine**, in Rust, runs bounded strategies. Each step asks a judge one
  question: facts (code), System One, System Two or a person. Every judge takes
  the same question and returns the same answer. Models are configuration;
  only each API format is code.
- **Plugins**, in TypeScript, are the behaviors: they hook into harness events,
  ask the engine questions, and act on the harness through generic actions.
- **Adapters**, one per harness, translate that harness's hooks into standard
  events and carry out actions.
- **Callers**, any program, send a question with its context through
  `chauffeur.navigate`. Rule files, which are data, decide how to answer.

Shipped content (models, facts, rules) lives apart from the engine, in
`distribution/`, and loads through the same path as the user's own.

The slices are:

1. a generic state machine that is playable in the browser;
2. `navigate`: the protocol, the judge contract, facts, rule files, the
   runtime, the daemon and the TypeScript client;
3. System One models: Jev, SubQ and GLiDE;
4. the first System Two models on the same path;
5. plugins and adapters: the event bus, the SDK, the OpenCode adapter and the
   plugins that reach parity with `main`;
6. release and cutover: v2 replaces v1 as the shipped Chauffeur.

The work proceeds one slice at a time, with review before advancing.

```mermaid
flowchart LR
    S1["1 Core<br/>after, proofs, playground"] --> S2["2 navigate<br/>protocol, judges, facts,<br/>runtime, daemon, client"]
    S2 --> S3["3 System One<br/>Jev, SubQ, GLiDE"]
    S3 --> S4["4 System Two<br/>OpenAI, Anthropic"]
    S4 --> S5["5 Plugins and adapters<br/>parity with main"]
    S5 --> S6["6 Release<br/>1.0.0-rc, cutover"]
```

## Production and parity with main

Users run v1, released from `main`. v2 changes nothing they install until
cutover, and `main` keeps receiving features and fixes meanwhile.

```mermaid
flowchart LR
    Main["main (v1)<br/>features and fixes"] --> CI["check.yml<br/>fmt, clippy, tests, package smoke"]
    CI --> Brew["Homebrew release<br/>what users run"]
    Main -. "ported by behavior<br/>tracked below" .-> V2["v2 branch<br/>slices 1–6"]
    V2 --> RC["1.0.0-rc.N<br/>next channel"]
    RC --> Cut["Cutover<br/>v2 merged into main"]
```

| Concern | Decision |
|---|---|
| Porting | Each `main` feature is re-expressed in v2's architecture, not merged: v2's tree replaces v1's, so merges would conflict on every removed file |
| Tracking | The parity table lists every `main` commit after the branch point; the watermark names the last one reviewed |
| Evidence | A ported feature passes `main`'s cases for it, replayed against v2 |
| Version mixing | The daemon, plugin and adapter version handshake keeps v1 and v2 from running together |
| User state | `~/.config/chauffeur` is migrated, or read compatibly, at cutover |
| After cutover | `v1` is a branch for hotfixes only |

### Parity table

Branch point `73bc29e`. Watermark: `e7fbabf`.

| `main` commit | v2 home | Slice | Evidence from `main` |
|---|---|---|---|
| `f2b087b` judge interface, facts judge | The `match` fact in `distribution/facts/`; `judge.json` routes become step judge names | 2 | `judges/facts/src/lib.rs` tests |
| `bb41c9a` context renewal, session warming | The `context-renewal` and `warming` plugins; the adapter carries out `session.compact` and `session.warm` | 5 | `routing.rs`, `renewal.test.ts`, `warming.test.ts` |
| `22e5391`, `36f6253` skill safety review | The skills plugin's screen-then-review strategy | 5 | `sources/tests.rs` vetting cases |
| `3a18675` Skilld discovery | The skills plugin's remote sources; fetches run in the plugin, never on the daemon's coordinator (audit A4) | 5 | `cli/tests/skills.rs` |
| `c43ecd6` read guard, RPC starvation fix | The guard plugin's `before` hooks; runtime admission permits (audit A3) | 2, 5 | `read-guard-probes.json`, `rpc-starvation.test.ts` |
| `9b8e917` rules | Rule files in `distribution/rules/` | 5 | `capabilities/rules/tests/rules.rs` |
| `73bc29e` Homebrew release, CI | Release scripts and `check.yml` in v2 | 6 | `release.test.mjs`, `smoke.mjs` |
| `f54db24`, `d66a385`, `e7fbabf` docs | Documentation only | — | — |
| In progress on `main`: GLiDE provider, confidence escalation | `glide.json` over the `systemone` format; escalation is a strategy: System One with `model: "jev"`, then `model: "glide"`, then System Two | 3 | `judges/glide/tests/wire.rs`, `docs/judges.md` escalation cases |

Context renewal on v2:

```mermaid
sequenceDiagram
    participant O as OpenCode
    participant A as OpenCode adapter
    participant D as Chauffeur daemon
    participant P as context-renewal plugin
    O->>A: session resumes
    A->>D: event session.resumed, with idle time and tokens
    D->>P: session.resumed
    P->>P: idle beyond the cache, and over half the window
    P->>D: action session.compact, with the summary model
    D->>A: session.compact
    A->>O: check the revision, then compact
```

## Architecture

```mermaid
flowchart TB
    subgraph TS["@fdcn/chauffeur: TypeScript SDK"]
        Client["/client"]
        Judge["/judge"]
        PluginSDK["/plugin"]
        AdapterSDK["/adapter"]
        Harness["/harness"]
    end
    subgraph Engine["Rust engine"]
        Daemon["daemon<br/>socket, event bus, plugin supervisor"]
        Run["protocol, runtime, judges, facts"]
        Core["core<br/>after, rounds, events"]
    end
    Dist[("distribution/<br/>models, facts, rules")]
    TS -- "Unix socket or stdio" --> Daemon
    Daemon --> Run
    Run --> Core
    Dist --> Daemon
```

| Kind | What it is | Written by | Examples |
|---|---|---|---|
| Rule file | Data: `on`, `when`, `judges`, `asks`, `strategy`, `then` | Chauffeur, an integration, the user or a project | `conventional-commits`, `sourcefed/triage`, `opencode-history/recall` |
| Fact | A JavaScript module that answers a question without a model | Chauffeur, the user, or a project (pure only) | `match`, `select`, `bm25`, a user's `notes` |
| Plugin | A TypeScript program that hooks events, asks questions and acts | Chauffeur, the user, anyone | `skills`, `context-renewal`, `warming`, `guard` |
| Caller | Any program that sends a question and its context | Anyone | sourcefed's daemon, opencode-history, hook scripts |
| Adapter | Per-harness glue: standard events out, actions in | Chauffeur, one per harness | `adapters/opencode`, `adapters/pi` |

Policy lives in rule files, facts and plugins. Adapters and callers hold no
judging logic: an adapter translates, and a caller sends its question and acts
on the intents it gets back.

### Dependency direction

```mermaid
flowchart TB
    Core["crates/core<br/>Generic plan state machine"]
    O11y["crates/o11y<br/>Observes core events"] --> Core
    Protocol["crates/protocol<br/>Requests, rules, events, actions"] --> Core
    Judges["crates/judges<br/>Questions, answers, confidence, formats"] --> Core
    Judges --> Facts["crates/facts<br/>QuickJS sandbox"]
    Runtime["crates/runtime<br/>Drives rounds, deadlines, redaction"] --> Core
    Runtime --> Judges
    Runtime --> O11y
    Daemon["crates/daemon<br/>Socket, rules, event bus, plugins"] --> Runtime
    Daemon --> Protocol
    Playground["playground<br/>WASM + interactive tabs"] --> Protocol
```

`crates/core` has no crate dependencies: no async runtime, HTTP, serde, UI,
logging or plugin system. Error types use the standard library. Other crates
consume core's event data. Adding crates does not by itself increase the
shipped binary: linked code and dependencies determine size.

### Why the engine is Rust

Measured on the development machine:

| | Result |
|---|---|
| One message over the Unix socket, 20 KB of JSON | 0.023 ms p50, 0.232 ms p99 |
| One Jev call (opencode-history's recall replay) | 278 ms p50 |
| v1's Rust daemon at rest | 66 MB |
| Idle Bun, idle Node | 10 MB, 37 MB |

| Concern | Rust's part |
|---|---|
| Safety | The deciding reason. The daemon parses untrusted input (the socket, project rules, project facts), holds model keys and redacts evidence. Rust gives memory safety there, `serde` rejects malformed input, and Kani and Miri prove strategies finish within their limits |
| Speed | Not decisive: a hop costs 0.03 ms against a 278 ms model call |
| Resource use | Decided by design more than language: lazy plugins, idle shutdown and bounded caches |

Plugins, adapters and the SDK are TypeScript. gdp-ts types the places where
a check must come before an action (see [Proofs](#proofs)).

## `chauffeur.navigate`

Callers use one operation, `chauffeur.navigate`: the decision a chauffeur
makes before steering. A request names an event and carries its context;
every rule whose `on` and `when` match answers it, and the caller receives
their intents:

```ts
import { connect } from "@fdcn/chauffeur/client"

const { intents } = await connect().navigate(
  { on: "recall", evidence: { message }, candidates: { p1: "…", p4: "…", p7: "…" } },
  { signal: AbortSignal.timeout(1_500) },
)
// intents → [{ rule: "opencode-history/recall", attach: ["p4", "p1"] }]
```

```mermaid
flowchart LR
    Callers["Callers<br/>sourcefed, opencode-history,<br/>plugins, scripts"] -- "event and context" --> Daemon["Chauffeur daemon"]
    Rules[("Rule files<br/>shipped, integrations,<br/>user, project")] --> Daemon
    Daemon --> Strategy["Each matching rule's strategy<br/>facts, System One, System Two"]
    Strategy -- "intents" --> Callers
```

- **The strategy is the rule's.** A rule can screen with facts, classify with
  System One and review only what System One leaves unsure with System Two.
  Changing the strategy changes a rule file, never a caller.
- **No intent means the caller's default.** A rule that matches nothing in its
  `then`, or whose judges fail, returns no intent: sourcefed then delivers the
  event, and recall attaches nothing.
- **An inline `strategy` replaces `on`** for scripts, plugins, the playground
  and tests, with its own `judges` and `asks`. It runs only that strategy and
  returns its verdict. The SDK's `ask` and `strategy` send inline strategies.

### sourcefed: one triage for every host

sourcefed's daemon navigates `integration_event` before it queues an event for
a target, so OpenCode, Pi, MCP and CLI all receive the same filtered stream.
sourcefed installs `sourcefed/triage`, a System One screen that withholds
confident noise; the user tunes or extends it in JSON.

```mermaid
sequenceDiagram
    participant S as sourcefed daemon
    participant C as Chauffeur daemon
    participant H as Any host (OpenCode, Pi, MCP, CLI)
    S->>C: navigate integration_event, with event, monitor and thread
    C->>C: Run sourcefed/triage and any other matching rule
    C-->>S: withhold, or no intent
    S->>H: Queue the event unless withheld
```

With no Chauffeur, an error, or no answer within 3 seconds, sourcefed
delivers, as v1's gate does.

### opencode-history: recall

opencode-history searches its own index when the user sends a message, then
navigates `recall` with the message and the passages as candidates. Its rule,
`opencode-history/recall`, ranks them, and opencode-history attaches the
passages the `attach` intent names.

### One protocol, many bindings

`navigate` is a Chauffeur operation, not any host's. Its request and result
are defined once, in `crates/protocol/schema/navigate.json`, and every binding
forwards them to the daemon socket unchanged:

| Binding | Caller | Mechanism |
|---|---|---|
| `@fdcn/chauffeur/client` | Any TypeScript program | `connect().navigate(request)` over the socket |
| `chauffeur navigate` | Hook scripts, Git hooks, CI on the same machine | One request on stdin; the JSON-RPC messages on stdout |
| `crates/protocol` | Rust programs | The same types the daemon uses |
| An adapter | Its host's own plugins, as a convenience | The host's plugin-to-plugin mechanism, such as OpenCode plugin RPC |

Adding a host or a language adds a binding, never a new request format. The
schema's fixtures run in the protocol crate's tests and in each binding's
tests.

### JSON-RPC 2.0

Every message to and from the engine is JSON-RPC 2.0, one message per line,
over two transports:

| Transport | Used by | Why |
|---|---|---|
| The Unix socket | Clients, the CLI, adapters | Independent programs reach the daemon; the socket's `0700` directory is the access control |
| stdio | Plugins the daemon starts | A plugin's lifetime is tied to the daemon's, with no socket permissions to manage |

The same messages travel on both. A request with an `id` gets a result or an
error; a message without one is a notification, used for streamed events.
`$/cancelRequest` with a request's `id` cancels it, and closing a connection
cancels everything it carried. JSON-RPC is the standard LSP and MCP use: it
needs nothing beyond `serde_json`, and its messages stay readable when
debugging.

| Method | Direction | Carries |
|---|---|---|
| `navigate` | Caller → daemon | A request; `event` notifications stream back, then the intents or verdict |
| `emit`, `before` | Adapter → daemon | A harness event; `before` returns the edits plugins made |
| `act` | Daemon → adapter | An action, with the revision it applies to |
| `start` | Daemon → plugin | Discovery or run mode, options and the actions available |
| `hooks` | Plugin → daemon | The events the plugin hooks with `on` and `before` |
| `event`, `before` | Daemon → plugin | A hooked event; `before` returns edits |
| `act`, `navigate` | Plugin → daemon | An action, or a question for the engine |

### Stateless invocation

A navigation has a caller, request ID, an event or inline strategy, bounded
evidence and candidates, and a deadline. It runs without an agent session or
prior history; the caller supplies the evidence it wants considered. Intents
and transition events return to that caller. The only state kept between
navigations is a rule's cooldown. Plugins keep their own state.

## Judges

Every judge takes one question and returns one result. Judges come in four
tiers. A rule names its judges in `judges`, each with a tier, an optional
model and a confidence, and a step's `judge` names one of them or a tier:

| In a rule | Answered by |
|---|---|
| `"quick": { "tier": "system-one", "confidence": 0.7 }` | The tier's models, in the user's fallback order |
| `"careful": { "tier": "system-one", "model": "glide", "confidence": 0.7 }` | GLiDE; the tier's fallback order applies if GLiDE is unavailable or not configured, and the run's events say so |
| A step with `"judge": "facts"` | The facts tier, with no model and no confidence |

Naming the model lets one strategy use two models of the same tier, such as
Jev for a fast screen and GLiDE, more accurate but slower, for what Jev leaves
unsure. A rule shared with someone who lacks GLiDE still runs, on their
System One.

### Questions

A question is instructions and a list of possible outcomes. Nothing else
reaches a judge:

```jsonc
{ "instructions": "What should the agent do with this message?",
  "outcomes": ["act", "ignore", { "value": "ask-user", "description": "Unclear; the user decides" }],
  "evidence": ["thread", "event"] }
```

- 2 to 64 outcomes, each unique. An outcome is a string, or a `value` with a
  `description` that the model sees.
- The list's order is meaningful: it is the order of a scale, when the
  outcomes are one.
- `unsure` and `failed` are reserved and cannot be outcomes.
- `evidence` names the request's evidence blocks the judge sees.

The SDK's question functions build this shape, using Jev's names:

| Function | Builds | Example |
|---|---|---|
| `noul(instructions)` | `outcomes: ["yes", "no"]` | "Does this commit message break Conventional Commits?" |
| `choice(outcomes, instructions)` | `outcomes` as given | "What should the agent do with this message?" |
| `score(levels, instructions)` | `outcomes` as given, lowest first | "How hard is the user's request?" |
| `question({ instructions, outcomes })` | The primitive the others use; anyone can write new functions over it | `question({ instructions, outcomes: ["none", "minor", "major", "irreversible"] })` |

### Results

A judge either answers or fails. Failure is not an outcome:

| Side | Carries |
|---|---|
| Answer | `outcome`: one of the question's outcomes, or `unsure`. `p`: a probability for every outcome. `level`: the probability-weighted position in the list, from 0 (first) to n−1 (last) |
| Failure | Why: `unavailable`, `rate_limited`, `timeout`, `invalid` or `deadline` |

```json
{ "answer": { "outcome": "medium", "p": { "low": 0.12, "medium": 0.45, "high": 0.43 }, "level": 1.31 } }
{ "failed": "timeout" }
```

`level` is meaningful when the outcomes are a scale: `level ≥ 1` reads as
"medium or higher" even when no single level is likely enough. Every tier
answers every question, so moving a step between tiers changes only its
`judge`.

### Confidence

The rule, not the model, decides when an answer is sure enough. A named
judge's `confidence` is how likely its top outcome must be, from 0 to 1:

| `confidence` | Outcome |
|---|---|
| `0.7` | The likeliest outcome when its probability is 0.7 or more; otherwise `unsure` |
| `{ "yes": 0.6, "no": 0.95 }` | Per outcome: here `yes` needs 0.6 and `no` needs 0.95. The map must cover every outcome of every question the judge is used for, checked when the rule loads |
| none | The likeliest outcome; a tie is `unsure` |

```text
confidence 0.7 on noul "Does this commit message break Conventional Commits?"
  "fixed stuff"              yes 0.94, no 0.06 → yes
  "feat(cli): add navigate"  yes 0.06, no 0.94 → no
  "Update parser.rs"         yes 0.58, no 0.42 → unsure
  Jev timed out                                → failed: timeout
```

```text
confidence { yes: 0.6, no: 0.95 } on noul "Could this command cause irreversible harm?"
  "rm -rf build/"   yes 0.71, no 0.29 → yes
  "ls -la"          yes 0.02, no 0.98 → no
  "git clean -fd"   yes 0.12, no 0.88 → unsure, so the user is asked
```

A failure stays a failure whatever the confidence, so it never reads as `no`.
The runtime applies confidence the same way for every tier, in thousandths so
comparisons stay exact, and models never see it. Each judge's confidence is
tuned against labelled cases.

### Tiers

| Tier | Answers with | Shipped |
|---|---|---|
| `facts` | JavaScript functions in a sandbox: no model and no credentials | `match`, `select`, `bm25`, `embedding` |
| `system-one` | A fast classifier, one batched call per round | `jev` (default), `subq`, `glide` |
| `system-two` | A reasoning model, with a schema-checked answer that cites its evidence | Disabled until reviewed (slice 4) |
| `person` | The user, asked through the adapter | Each adapter |

### How a step reaches a model

A step reaches the code that answers it through four layers. Each layer knows
only the one below it, and all four tiers are built the same way:

```mermaid
flowchart LR
    Step["Step<br/>judge: quick"] --> Judge["Named judge<br/>tier, model?, confidence"]
    Judge --> Tier["Tier<br/>models in fallback order"]
    Tier --> Model["Model entry<br/>name, format, settings"]
    Model --> Format["Format<br/>question in, probabilities out"]
    Format -- "answer or failure" --> Confidence["confidence → outcome"]
```

| Layer | Defined in | Example |
|---|---|---|
| Named judge | A rule's `judges`, or `judge()` in the SDK | `"quick": { "tier": "system-one", "confidence": 0.7 }` |
| Tier | `models.json` `tiers` | `"system-one": ["jev", "subq"]` |
| Model entry | `models.json` `models` | `"jev": { "format": "systemone", "endpoint": "…", "key": { "env": "JEV_KEY" } }` |
| Format | Rust in `crates/judges` | `systemone`, `openai`, `anthropic`, `facts`, `person`, `placeholder` |

Every format implements one interface. A model answers every question it
received in a round with one call, and the format decides how to ask its
model from the question's outcomes:

```rust
pub trait Format {
    fn answer_all(&self, questions: &[Question], evidence: &Evidence) -> Vec<Result<Probabilities, Failure>>;
}

pub enum Failure {
    Unavailable, // the tier's next model answers
    RateLimited, // the tier's next model answers
    Timeout,     // the tier's next model answers
    Invalid,     // the step fails
}
```

| Format | Speaks |
|---|---|
| `systemone` | The `/v1/systemone` API that Jev, SubQ Snap and Fastino GLiDE share: its `noul` type for `["yes", "no"]`, and `choice`, which returns every outcome's probability, for the rest. GLiDE's entry sets its reading of `score` as a level |
| `openai` | OpenAI-compatible chat completions, with a JSON schema whose `enum` is the outcomes; covers local servers such as Ollama |
| `anthropic` | Anthropic Messages with the same schema |
| `facts` | Runs a fact in `crates/facts` |
| `person` | A question to the user through the adapter |
| `placeholder` | Recorded or fixed answers, standing in until the real model is configured |

An outcome outside the question's list fails the step as `invalid`.
`crates/judges` turns each tier's models into one core `Judge`, so core sees a
judge per tier and model, such as `system-one:glide`, and nothing below it.

### Models are configuration

A model is registered the way a user registers a model provider anywhere
else: a name, a built-in API format, and the endpoint, model ID and credential
to use.

```jsonc
// ~/.config/chauffeur/models.json
{
  "models": {
    "local-qwen": { "format": "openai", "endpoint": "http://127.0.0.1:11434/v1", "model": "qwen3:4b" },
    "reviewer":   { "format": "anthropic", "model": "claude-sonnet-5-5",
                    "key": { "env": "ANTHROPIC_API_KEY" }, "timeout_ms": 20000 }
  },
  "tiers": {
    "system-one": ["jev", "subq", "local-qwen"],
    "system-two": ["reviewer"]
  }
}
```

- A step's `model` names an entry. `distribution/models/` ships `jev`, `subq`
  and `glide` as the same JSON. The user's file adds models and overrides
  shipped ones by name. Changes take effect without a restart.
- `tiers` lists models in fallback order. The next model answers only when
  one is unavailable, rate-limited or timed out, never because of what it
  answered. Escalating to another model or tier is the strategy's job.
- Models load only from shipped and user configuration, never from a
  project's `.chauffeur/`. A model entry is a network endpoint, so a cloned
  repository could otherwise send evidence wherever it chose.
- A key is named, as an environment variable or a keychain entry, and is read
  only by `crates/judges`. It never reaches rule files, callers, plugins or
  audit.
- A shipped model is used once its key is present; a System Two model also
  needs the user to turn it on.

### Placeholders

A `placeholder` model stands in for a tier, or for one fact, until the real
one is configured. Rules do not change when it is replaced; only `models.json`
does.

```jsonc
// models.json for tests and the playground
{
  "models": {
    "one-stand-in":    { "format": "placeholder", "answers": "fixtures/triage-answers.json" },
    "two-stand-in":    { "format": "placeholder", "otherwise": { "act": 0.8, "ignore": 0.2 } },
    "person-stand-in": { "format": "placeholder", "otherwise": { "yes": 1, "no": 0 } }
  },
  "tiers": {
    "system-one": ["one-stand-in"],
    "system-two": ["two-stand-in"],
    "person":     ["person-stand-in"]
  }
}
```

```jsonc
// fixtures/triage-answers.json
{ "answers": [
  { "evidence": { "event": "thanks, looks good 👍" },     "p": { "yes": 0.52, "no": 0.48 } },
  { "evidence": { "event": "@agent bump retries to 5" }, "p": { "yes": 0.94, "no": 0.06 } },
  { "evidence": { "event": "deploy is on fire" },        "failed": "timeout" }
] }
```

- `answers` is matched by evidence: an entry answers a question whose evidence
  contains every block the entry lists, with the same content. The first
  match wins, so one file serves every rule that asks about the same input.
- `otherwise` answers what no entry matches; without it, the step fails as
  `unavailable`, so a gap in the recorded answers shows.
- Every placeholder answer is marked in events and in `strategy show`, so it
  never passes for a real model's.
- Recorded answers can be real ones, such as opencode-history's Jev scores, to
  tune a rule's confidence before it runs live.

```mermaid
flowchart TB
    subgraph Slice2["Slice 2: every tier answers"]
        F["facts → match, select"]
        One["system-one → placeholder"]
        Two["system-two → placeholder"]
        P["person → placeholder"]
    end
    One -- "slice 3" --> Jev["jev, subq, glide"]
    Two -- "slice 4" --> Rev["openai, anthropic"]
    P -- "slice 5" --> Host["adapter asks the user"]
```

## Facts

A fact is a JavaScript module that answers a question without a model. It
runs in `crates/facts`, an embedded QuickJS sandbox. Shipped facts and the
user's own have the same shape; a user's fact replaces a shipped one of the
same name.

```mermaid
flowchart LR
    Step["Facts step<br/>ask: notes.bm25"] --> Find["Find the fact<br/>user, project, shipped"]
    Find --> Engine["QuickJS<br/>memory and time caps"]
    Host[("Host modules<br/>files, SQLite, loopback HTTP")] --> Engine
    Engine --> Result["Answer or failure"]
```

A fact module exports a default function, named functions, or both:

```js
// ~/.config/chauffeur/facts/notes.js
import { query } from "chauffeur:sqlite"

// "notes.bm25": BM25 over the user's own notes database
export function bm25({ evidence, args }) {
  const rows = query("~/notes/index.db",
    "SELECT id FROM notes WHERE notes MATCH ? ORDER BY bm25(notes) LIMIT ?", [evidence.message, args.top ?? 10])
  return { ranked: rows.map((row) => row.id) }
}

// "notes": is the message about something already in the notes?
export default function ({ evidence }) {
  const [hit] = query("~/notes/index.db", "SELECT 1 FROM notes WHERE notes MATCH ? LIMIT 1", [evidence.message])
  return { p: { yes: hit ? 1 : 0, no: hit ? 0 : 1 } }
}
```

A facts step names the fact and its arguments as its `ask`:

```jsonc
{ "id": "related", "judge": "facts", "ask": { "notes.bm25": { "top": 10 } } }
{ "id": "is-slack", "judge": "facts", "ask": { "match": { "field": "event.source", "equals": "slack" } } }
```

| A fact receives | A fact returns |
|---|---|
| `evidence`, the request's named blocks | `{ p }` over the outcomes, usually 1 and 0 |
| `candidates` | `{ ranked: [...] }`: candidate names, best first |
| `outcomes` of earlier steps, with their `p` and `level` | Throws: the step fails as `invalid` |
| `args`, from the step | |

| Source | Folder | Host modules |
|---|---|---|
| Shipped | `distribution/facts/` | `chauffeur:fs` (read), `chauffeur:sqlite` (read-only), `chauffeur:http` (loopback only) |
| User | `~/.config/chauffeur/facts/` | The same |
| Project | `.chauffeur/facts/` | None: pure functions of their inputs |

### Shipped facts

| Fact | Does |
|---|---|
| `match` | Conditions on evidence fields: `equals`, `one_of`, `contains`, `pattern` (glob), `at_least`, `at_most` (a number, or another field times `times`); combined with `all` and `any`; `cases` answers a choice |
| `select` | Combines earlier outcomes: `first` settled in a list, `top` N, all at or above `min`, `majority` or `mean`; `by` names the outcome whose probability ranks, such as `yes` |
| `bm25` | Ranks candidates against a query |
| `embedding` | Ranks candidates by similarity from the local embedding service |

v1's context-renewal rule, as a `match`:

```jsonc
{ "id": "renew", "judge": "facts", "ask": { "match": { "all": [
  { "field": "idle_seconds",   "at_least": { "field": "cache_seconds" } },
  { "field": "context_tokens", "at_least": { "field": "context_window", "times": 0.5 } }
] } } }
```

A field the evidence lacks fails the step as `invalid`, never `no`.

### Limits

| Concern | Decision |
|---|---|
| Runaway code | Each call has a memory cap (16 MiB by default) and an interrupt at the step's deadline; overrunning either fails the step |
| A missing fact | `unavailable`, so the facts tier's next model answers: a placeholder can stand in |
| Names | A fact cannot be named `instructions`, `outcomes` or `evidence` |
| Project facts | Run with no host modules. QuickJS is not a hardened sandbox, so a memory-safety bug in it could let a repository's code escape; the caps and the absence of host modules keep that risk small, and it is accepted |
| Engine | `rquickjs` in `crates/facts`; core stays dependency-free |

## Strategies

A strategy is a list of steps, built from four pieces. Chauffeur has no list
of built-in strategies: any combination of the pieces is one.

| Piece | What it does |
|---|---|
| Step | Asks one judge one question; its confidence turns the answer into an outcome |
| `after` | Waits for earlier steps, then runs the step or skips it by their outcomes |
| `for_each` | Copies a step once per candidate, in one round |
| Earlier outcomes | A step may read earlier steps' outcomes, `p` and `level` as its evidence |

### `after`

A step waits until every step its `after` names has finished. Then it runs if
every `all` entry is met and, when there are `any` entries, at least one of
them is; otherwise it is skipped. A step without `after` runs in round 1.

```jsonc
"after": ["fast", "slow"]                                     // all: wait for both, however they finished
"after": [{ "step": "fast", "when": ["unsure", "failed"] }]   // all: run only if fast was unsure or failed
"after": { "all": [ … ], "any": [ … ] }                       // every all entry, and at least one any entry
```

- A string entry is met once that step finishes, even when it was skipped.
- An object entry is met only when the step's outcome is in `when`. `failed`
  in `when` matches a step whose judge failed, for any reason. A skipped step
  never matches.
- `relevant[*]` names every copy `for_each` made of `relevant`. Inside a copy,
  `relevant[item]` names the same candidate's copy.
- `after` names only earlier steps, so a strategy cannot loop. Every strategy
  has a known maximum of rounds and calls before it starts.

Combining answers is a step too: the `select` fact reads earlier outcomes and
settles to one. A new way to combine is a new fact. The protocol lowers
`for_each`, wildcards, named judges and named asks to plain core steps, so
core runs every strategy alike.

| Strategy | Built from |
|---|---|
| Escalate | A step `after` an earlier one, `when` it is `unsure` or `failed` |
| Rank | A `for_each` step, then `select` with `top` and `min` |
| Collect all that pass | A `for_each` step, then `select` with `min` |
| Check the survivors | Two `for_each` steps, the second `after` `first[item]` `when` `yes` |
| Vote | Parallel steps, `select` with `majority`, and a review `after` the vote `when` `unsure` |

Escalating from Jev to GLiDE, then to System Two:

```jsonc
"judges": {
  "quick":    { "tier": "system-one", "model": "jev",   "confidence": 0.7 },
  "careful":  { "tier": "system-one", "model": "glide", "confidence": 0.7 },
  "reasoner": { "tier": "system-two",                   "confidence": 0.7 }
},
"strategy": [
  { "id": "fast",   "judge": "quick",    "ask": "needs-agent" },
  { "id": "slow",   "judge": "careful",  "ask": "needs-agent", "after": [{ "step": "fast", "when": ["unsure", "failed"] }] },
  { "id": "review", "judge": "reasoner", "ask": "needs-agent", "after": [{ "step": "slow", "when": ["unsure", "failed"] }] },
  { "id": "answer", "judge": "facts", "ask": { "select": { "first": ["review", "slow", "fast"] } },
    "after": ["fast", "slow", "review"] }
]
```

```text
"thanks, looks good 👍"
round 1   fast    jev    yes 0.52, no 0.48      → unsure
round 2   slow    glide  yes 0.18, no 0.82      → no
          review  skipped: slow was not unsure or failed
          answer  fast, slow and review have finished → asked
round 3   answer  select first                  → no
```

### Viewing a strategy

`chauffeur strategy show` prints a rule's strategy as a tree, with each
step's round, its model as the user's configuration resolves it, its `after`
and its confidence:

```text
$ chauffeur strategy show opencode-history/recall
opencode-history/recall                        on recall
├─ round 1  helps × candidates  system-one → jev
└─ round 2  best               facts select   top 3, min 0.7, from helps
then  attach best
at most 2 rounds · 1 model call · 31 candidates at most
```

`--file` shows an unsaved rule, and an unknown or unconfigured model is marked
where it appears. The playground draws the same tree and steps through it with
recorded answers.

## Rule files

A rule file needs no code: dropping it into a rules folder is the whole
install.

| Field | Meaning |
|---|---|
| `on`, `when` | The event that triggers the rule, and a cheap filter on it |
| `judges` | Named judges: a tier, an optional model and a confidence |
| `asks` | Named questions, so steps that ask the same question share it |
| `strategy` | The steps, in order: each with an `id`, a `judge` (a named judge or a tier), an `ask` (a named question, one written inline, or a fact), and `after` |
| `then` | Intents by outcome, first match wins; no match returns no intent |

Names in `judges` and `asks` are referred to only by a step's `judge` and
`ask`. Step IDs are referred to only by `after`, `select` and `then`.

Rules load from four sources, and edits apply without a restart:

| Source | Folder | System Two |
|---|---|---|
| Shipped | `distribution/rules/` | Allowed |
| Integration | `~/.config/chauffeur/rules/<integration>/`, installed by the integration | Allowed |
| User | `~/.config/chauffeur/rules/` | Allowed |
| Project | `.chauffeur/rules/`, from the Git worktree root down to the session's directory | Only where `~/.config/chauffeur` allows it for the project |

A rule's ID is its path under its folder, such as `sourcefed/triage`. The user
tunes an integration's rule by editing its file; reinstalling the integration
keeps a file the user changed.

### Commit messages

```jsonc
// .chauffeur/rules/conventional-commits.json
{
  "on": "tool.after",
  "when": { "tools": ["shell"] },
  "judges": { "quick": { "tier": "system-one", "confidence": 0.7 } },
  "strategy": [
    { "id": "nonconventional", "judge": "quick",
      "ask": { "outcomes": ["yes", "no"], "evidence": ["tool_call"],
               "instructions": "Does this call create a git commit whose message does not follow Conventional Commits?" } }
  ],
  "then": [{ "if": { "nonconventional": "yes" },
             "message.append": { "parts": [{ "text": "Use Conventional Commits, such as `feat(scope): short description`." }], "delivery": "now" } }],
  "cooldown_seconds": 120
}
```

A rule on a harness event names [actions](#actions) in its `then`, and the
daemon passes them to the adapter. A rule on a caller's event, such as
`integration_event` or `recall`, returns intents the caller defines, such as
`withhold` and `attach`.

### Event triage

sourcefed installs this rule with its System One screen. This user added the
System Two review for Slack messages that System One leaves unsure or fails
on.

```jsonc
// ~/.config/chauffeur/rules/sourcefed/triage.json
{
  "on": "integration_event",
  "judges": {
    "quick":    { "tier": "system-one", "confidence": 0.7 },
    "reasoner": { "tier": "system-two", "confidence": 0.6 }
  },
  "asks": {
    "needs-agent": { "outcomes": ["yes", "no"], "evidence": ["thread", "event"],
                     "instructions": "Does the new event ask the agent to do or change something?" }
  },
  "strategy": [
    { "id": "screen",   "judge": "quick", "ask": "needs-agent" },
    { "id": "is-slack", "judge": "facts", "ask": { "match": { "field": "event.source", "equals": "slack" } } },
    { "id": "review",   "judge": "reasoner",
      "ask": { "outcomes": ["act", "ignore"], "evidence": ["thread", "event"],
               "instructions": "Should the agent act on the new message? Cite the evidence." },
      "after": [{ "step": "screen", "when": ["unsure", "failed"] }, { "step": "is-slack", "when": ["yes"] }] }
  ],
  "then": [
    { "if": { "screen": "no" }, "withhold": "event" },
    { "if": { "review": "ignore" }, "withhold": "event" }
  ]
}
```

Two rounds at most: `review` is skipped when the screen says `yes` or `no`, or
the event is not from Slack. Every other outcome, including a failed review,
returns no intent, and sourcefed delivers.

### Per-turn recall

opencode-history installs this rule. The rule ranks the passages its search
returned; the code gathers them and attaches the result.

```jsonc
// ~/.config/chauffeur/rules/opencode-history/recall.json
{
  "on": "recall",
  "strategy": [
    { "id": "helps", "judge": "system-one", "for_each": "candidates",
      "ask": { "outcomes": ["yes", "no"], "evidence": ["message", "candidate"],
               "instructions": "Would this passage materially help the agent answer the message, beyond what it already says?" } },
    { "id": "best", "judge": "facts", "after": ["helps[*]"],
      "ask": { "select": { "from": "helps[*]", "top": 3, "min": 0.7, "by": "yes" } } }
  ],
  "then": [{ "attach": "best" }]
}
```

`select` ranks the copies by their probability of `yes`. All candidates share
one round, so Jev answers them in one call. An error or a missed deadline
attaches nothing.

### What a caller gets back

| Result | Meaning | `navigate` |
|---|---|---|
| Intents | Every matching rule ran | `intents`: each with its `rule`, possibly empty |
| Verdict | An inline strategy's steps finished | `verdict[id]`: `{ "answer": … }`, `{ "failed": "<reason>" }` or `"skipped"` |
| Unknown judge | An inline strategy names a tier with no model, or one its source may not use; nothing ran. A rule that does is refused when it loads | `judge_unavailable` error |
| Limit exceeded | The strategy or evidence is over a named limit; nothing ran | `limit_exceeded` error |
| Busy | This source's or the daemon's capacity is in use | `busy` error |
| Deadline | The deadline passed; unfinished steps are unanswered | `deadline` error, with the events so far |
| Unavailable | The daemon is not running | `unavailable` error |

## Plugins

A plugin is a long-running TypeScript program that hooks into events, asks
questions and acts on the harness. The daemon starts it, passes it the events
it hooks, and carries its actions to the adapter. Plugins are harness-agnostic:
the same plugin works in every harness whose adapter supports the actions it
uses.

### How a message reaches a plugin

```mermaid
sequenceDiagram
    participant U as User
    participant O as OpenCode
    participant A as OpenCode adapter
    participant D as Chauffeur daemon
    participant P as skills plugin
    U->>O: sends a message
    O->>A: prompt hook fires
    A->>D: event prompt.sent
    D->>P: prompt.sent, starting the plugin if needed
    P->>D: ask its strategy pick-skills
    D-->>P: review-pr, rust-style
    P->>D: edit: append skills review-pr, rust-style
    D->>A: append skills
    A->>O: skills attached to the prompt
```

### Events

Events are defined once, in `crates/protocol/schema/events.json`. Adapters
emit them; plugins and rules hook them by name.

| Event | When | Key fields |
|---|---|---|
| `session.created` | A session starts | `session`, `parent` |
| `prompt.sent` | The user sends a message | `session`, `revision`, `text`, `skills`, `model` |
| `response.finished` | The agent finishes a turn | `session`, `tokens` |
| `tool.before`, `tool.after` | Around a tool call | `session`, `tool`, `input`, `output` |
| `permission.asked` | The harness asks for permission | `session`, `action`, `resources` |
| `session.idle`, `session.resumed` | The user goes quiet, or comes back | `session`, `idle_seconds`, `cache_seconds`, `tokens` |

A plugin or caller may define its own events, namespaced to it, such as
`sourcefed.event`.

### Actions

Actions are every control a harness exposes, named by resource and verb:

| Resource | Verbs |
|---|---|
| `session` | `create`, `update` (`model`, `agent`, `title`, `permissions`), `compact`, `warm`, `interrupt`, `remove` |
| `message` | `append`: parts (`{ text }`, `{ skill }`, `{ file }`) with a delivery: `now` (into the running turn), `next` (before the next turn) or `wake` (resumes an idle agent) |
| `permission` | `reply` |
| `tool`, `skill`, `agent`, `model`, `mcp`, `command` | `list`, `add`, `update`, `remove` |

Each adapter declares the resource and verb pairs it supports, and a plugin
can check them. An unsupported action returns `unsupported`. Adding a harness
control means adding a verb to the schema and to the adapters that support
it.

### `on` and `before`

| Hook | Holds the harness | Can |
|---|---|---|
| `ctx.on(event, handler)` | Never: the event goes ahead at once | Call any action, any time |
| `ctx.before(event, handler, { budget, onTimeout })` | Until the handler returns or `budget` ms pass | Edit what is about to happen |

`before` exists for the points where acting late is acting too late:

| Event | A `before` handler can | `onTimeout` |
|---|---|---|
| `prompt.sent` | Append parts, edit the text, block it | `allow` |
| `tool.before` | Block it, ask the user, rewrite its input | `ask`, `block` or `allow` |
| `permission.asked` | Reply, or ask the user | `ask` |

`ask` hands the decision to the user. A guard against irreversible harm uses
`ask`, so a slow or failed judge never lets the action through silently. Host
denials always win over a plugin.

### Retries

| Kind of action | Examples | If it fails or arrives stale |
|---|---|---|
| Sets state | `session.update`, registry `add`, `update` and `remove` | Retried with backoff; the last write wins |
| One-shot | `session.compact`, `session.interrupt` | Retried once if the adapter was unreachable; dropped if stale |
| Append | `message.append` | Delivered `next` when the plugin set `late: "next"`; otherwise dropped once its turn has passed |
| Guard decision | Block, ask the user | Never retried; on timeout, `onTimeout` applies |

An adapter checks each action's `revision` against the harness before
carrying it out, so a stale action never lands on a newer state.

### Lifecycle

```mermaid
stateDiagram-v2
    [*] --> Discovered: installed or changed
    Discovered --> Stopped: hooks recorded
    Stopped --> Starting: a hooked event arrives, or eager
    Starting --> Ready: setup done
    Ready --> Ready: events in, actions out
    Ready --> Stopped: idle for 10 minutes, unless eager
    Ready --> Backoff: crashes
    Backoff --> Starting: after 1 s, 2 s, 4 s … up to 60 s
    Stopped --> [*]: removed from plugins.json
```

- **Registration.** The user lists plugins in `~/.config/chauffeur/plugins.json`.
  A project can never add a plugin.
- **Discovery.** When a plugin is installed or its files change, the daemon
  runs its `setup` once in discovery mode: `ctx.on` and `ctx.before` only
  record the hooks, and the plugin stops. The hooks are cached against the
  plugin's file hash. `setup` must register hooks before doing work with side
  effects.
- **Lazy by default.** The first event the plugin hooks starts it, and events
  queue until it is ready. An idle plugin stops after 10 minutes.
- **Eager.** `"chauffeur": { "eager": true }` in the plugin's `package.json`
  starts it with the daemon and keeps it running, for plugins with timers or
  their own connections.
- **Transport.** The daemon starts each plugin as a child process and speaks
  [JSON-RPC 2.0](#json-rpc-20) over its stdin and stdout, so a plugin never
  outlives the daemon.
- **Hot reload.** When a plugin's files change, the daemon reruns discovery
  and restarts the plugin; its next event reaches the new code. Plugins run
  from source on Bun, so nothing is compiled first.
- **Isolation.** A crash affects only that plugin. A slow plugin's `on` events
  queue to a bound; overflow drops the oldest and says so in an event.

### The SDK

One package, `@fdcn/chauffeur`, with subpath exports so bundlers keep only
what is imported:

| Import | Holds |
|---|---|
| `@fdcn/chauffeur/plugin` | `Plugin.define({ id, setup(ctx) })`; `ctx` holds only `on`, `before` and `options` |
| `@fdcn/chauffeur/judge` | `ask`, `judge`, `strategy`, and the question functions `question`, `noul`, `choice`, `score` |
| `@fdcn/chauffeur/harness` | Actions: `session`, `message`, `permission`, and the registries |
| `@fdcn/chauffeur/adapter` | `Adapter.connect({ harness, actions })` and `emit` |
| `@fdcn/chauffeur/client` | `connect().navigate(request)` |

`ctx` holds what is tied to the plugin's lifetime. Everything else is a
function to import, and `/judge` works the same from any program through the
client.

`ask` returns a result typed from its question:

```ts
type Result<O extends string> =
  | { ok: true; outcome: O | "unsure"; p: Record<O, number>; level: number }
  | { ok: false; failed: "unavailable" | "rate_limited" | "timeout" | "invalid" | "deadline" }
```

```ts
import { ask, choice, judge, noul, score } from "@fdcn/chauffeur/judge"

const quick = judge({ tier: "system-one", model: "jev", confidence: 0.7 })

const commit = await ask(noul("Does this commit message break Conventional Commits?"), { judge: quick, evidence })
if (commit.ok && commit.outcome === "yes") { /* "yes" | "no" | "unsure" */ }

const next = await ask(choice(["act", "ignore", "ask-user"], "What should the agent do with this message?"), { judge: quick, evidence })
const effort = await ask(score(["low", "medium", "high"], "How hard is the user's request?"), { judge: quick, evidence })
if (effort.ok && effort.level >= 1) { /* medium or higher */ }
```

A question function returns `{ wire, read }`: the generic question, and a
reader that types the reply. `question` is the primitive the others use, so a
plugin defines a new kind of question without any change to the engine. An
outcome outside the list fails as `invalid` at run time, and a comparison with
an impossible outcome fails to compile. A strategy's `when` accepts only the
outcomes its step can produce.

### Plugins from v1's features

Context renewal:

```ts
import { Plugin } from "@fdcn/chauffeur/plugin"
import { session } from "@fdcn/chauffeur/harness"

export default Plugin.define({
  id: "context-renewal",
  setup(ctx) {
    ctx.on("session.resumed", (e) => {
      if (e.idle_seconds >= e.cache_seconds && e.tokens.used >= e.tokens.window / 2)
        session.compact(e.session, { model: ctx.options.summaryModel })
    })
  },
})
```

Guard against irreversible harm:

```ts
import { Plugin } from "@fdcn/chauffeur/plugin"
import { ask, judge, noul } from "@fdcn/chauffeur/judge"

const guard = judge({ tier: "system-one", model: "jev", confidence: { yes: 0.6, no: 0.95 } })

export default Plugin.define({
  id: "guard",
  setup(ctx) {
    ctx.before("tool.before", async (e) => {
      if (e.tool !== "shell") return
      const harmful = await ask(noul("Could this command cause irreversible harm?"), { judge: guard, evidence: { command: e.input.command } })
      if (harmful.ok && harmful.outcome === "yes") e.block("This command may cause irreversible harm.")
      else if (!harmful.ok || harmful.outcome === "unsure") e.askUser("Chauffeur isn't sure this is safe.")
    }, { budget: 2_000, onTimeout: "ask" })
  },
})
```

Skill exposure, with its own strategy:

```ts
import { Plugin } from "@fdcn/chauffeur/plugin"
import { judge, noul, strategy } from "@fdcn/chauffeur/judge"

const quick = judge({ tier: "system-one", confidence: 0.7 })
const pick = strategy([
  { id: "needs", judge: quick, forEach: "candidates", ask: noul("Does this request need this skill?") },
  { id: "best", judge: "facts", after: ["needs[*]"], ask: { select: { from: "needs[*]", top: 4, min: 0.7, by: "yes" } } },
])

export default Plugin.define({
  id: "skills",
  setup(ctx) {
    ctx.before("prompt.sent", async (e) => {
      const best = await pick.run({ evidence: { message: e.text }, candidates: e.skills })
      if (best.ok) e.append(best.outcome.map((skill) => ({ skill })))
    }, { budget: 400, onTimeout: "allow" })
  },
})
```

Session warming:

```ts
import { Plugin } from "@fdcn/chauffeur/plugin"
import { session } from "@fdcn/chauffeur/harness"

export default Plugin.define({
  id: "warming",
  setup(ctx) {
    ctx.on("session.idle", (e) => setTimeout(() => session.warm(e.session), 270_000))
  },
})
```

`strategy(...).run` returns the last step's result. `ask` and `strategy` send
inline strategies through `navigate`, so limits,
confidence, model fallback, redaction and keys stay in the engine. Plugins
never hold model keys.

## Adapters

An adapter is a harness plugin built with `@fdcn/chauffeur/adapter`. It
emits standard events from the harness's hooks and carries out actions with
the harness's own API. It holds no policy.

```ts
import { Plugin } from "@opencode/plugin"
import { Adapter } from "@fdcn/chauffeur/adapter"

export default Plugin.define({
  id: "chauffeur",
  async setup(oc) {
    const chauffeur = await Adapter.connect({
      harness: "opencode",
      actions: {
        "session.compact": ({ session }) => oc.session.compact({ sessionID: session }),
        "session.update":  ({ session, model }) => oc.session.switchModel({ sessionID: session, model }),
        "message.append":  ({ session, parts }) => oc.session.synthetic({ sessionID: session, text: render(parts) }),
      },
    })

    await oc.session.hook("prompt", async (e) => {
      const edits = await chauffeur.before("prompt.sent", { session: e.sessionID, text: e.prompt.text, skills: await oc.skill.list() })
      applyEdits(e, edits)
    })
  },
})
```

The OpenCode adapter maps every resource and verb the OpenCode V2 plugin API
offers. It also exposes `navigate` to other OpenCode plugins through plugin
RPC.

## Proofs

| What | How |
|---|---|
| `after` and rounds in core | Kani: a step waits for its needs, decisions follow the rule, a run never stalls and finishes within one round per step |
| The runtime | Miri: deadlines, cancellation, late results, overflow and capacity |
| Shipped facts | Exhaustive tests over small inputs, run in the QuickJS sandbox: for example, `select` with `top` and `min` keeps at most `top` candidates, each at or above `min`, best first, for every input up to 4 candidates |
| Confidence | Kani: for every probability in thousandths and every valid confidence, an answer settles to exactly one outcome, edges inclusive |
| TypeScript checks before actions | gdp-ts: an adapter carries out an action only with a `RevisionIsCurrent<Session, Revision>` proof, and an action the host gates only with a `HostAllowed<Action>` proof. Its lint preset rejects forged proofs |

## Access, delivery and failure rules

- The daemon listens only on a Unix socket in a directory only the user can
  open (`chauffeur/` in the user's runtime directory, mode `0700`), so only the
  user's own processes connect. There is no TCP port and no token.
- Code from a repository runs only as a project fact: a pure function in the
  QuickJS sandbox, with no host modules and with memory and time caps.
  Plugins and models come only from the user's configuration.
- A cloned repository's rules are data the user did not write: they run under
  the same limits, and System Two stays off for them unless
  `~/.config/chauffeur` turns it on for that project.
- Callers and plugins are trusted code, as the user runs them. They reach
  Chauffeur only through `navigate` and the plugin protocol; payloads are
  never authority.
- A connection may carry several calls, each with its own `id`. A
  navigation's events arrive in order before its result. Each navigation's
  event buffer is bounded and overflow is an explicit event.
  `$/cancelRequest` cancels one call; closing the connection cancels all of
  them.
- One admission permit covers queued, waiting and active work. Capacity per
  source (shipped, integration, user and project rules, and inline
  strategies), strategy size, evidence bytes, fan-out, rounds, attempts and
  output size are checked before dispatch.
- Deadlines include queueing, preparation, every inference round and delivery.
  A disconnected caller cancels future work; a blocking provider call keeps
  its worker until it ends, and late results cannot commit actions.
- Redaction runs after evidence assembly and before every provider call, and
  also covers errors and audit metadata.
- Strategies return data and intents, never host effects. Only adapters act
  on a harness, after checking the revision; host denials cannot be
  overridden by a plugin.

## Core plan model

A plan is an ordered list of named steps. Each step carries a request of the
caller's type `Q`; the caller also chooses the outcome type `O`. Core does not
know what either means. Rule files and SDK strategies lower to this form: the
protocol resolves named judges and asks, `for_each` copies and wildcards
before core sees a plan.

```text
Step<Q, O>   = { id, judge: name, request: Q, after: After<O> }
After<O>     = { all: [Need<O>], any: [Need<O>] }
Need<O>      = { step: an earlier step's ID, when: [Outcome<O>] or none }
Outcome<O>   = Value(O) | Failed
StepState<O> = Pending | Asked | Settled(Outcome<O>) | Skipped
```

`Outcome` is core's result: `Value` is an answer's outcome, and `Failed` is
the failure side. In the engine, `O` is an outcome label, `unsure`, or a list
of candidate names from `select`; the runtime keeps each answer's `p` and
`level` beside it for `select`, evidence and events. `Skipped` is a machine
state, not an outcome.

### `after` in core

One rule decides every step, the one in [Strategies](#after):

- A step stays `Pending` until every step its `after` names is `Settled` or
  `Skipped`.
- It is then `Asked` if every `all` need is met and, when `any` has needs, at
  least one `any` need is met. Otherwise it is `Skipped`.
- A need without `when` is met by any finish. A need with `when` is met only
  by a settled outcome it lists; `Skipped` never matches.
- A step with an empty `after` is asked in round 1. Steps asked together share
  a round, and each judge answers its share of a round in one call.
- `after` names only earlier steps, so every plan finishes within one round
  per step.

The escalation from [Strategies](#strategies), in Rust:

```rust
let unsure_or_failed = [Value(Unsure), Failed];
let plan = Plan::new([
    Step::new("fast", "system-one:jev", ask),
    Step::new("slow", "system-one:glide", ask).after([Need::when("fast", unsure_or_failed.clone())]),
    Step::new("review", "system-two", ask).after([Need::when("slow", unsure_or_failed)]),
    Step::new("answer", "facts", select_first).after(["fast", "slow", "review"]),
]);
```

A step ID alone is a need without `when`, and `after_any` fills the `any`
list. A named judge lowers to a panel name of its tier and model, such as
`system-one:glide`. Rust, rule files and the SDK build the same steps.

### The `Judge` interface

A judge answers a step's request. Core defines the interface; it has no
dependencies and is synchronous:

```rust
pub trait Judge<Q, O> {
    fn judge(&self, request: &Q) -> Outcome<O>;
    // Defaults to one `judge` call per request; a batching provider overrides it.
    fn judge_all(&self, requests: &[&Q]) -> Vec<Outcome<O>>;
}
// Any `Fn(&Q) -> Outcome<O>` is a judge.
```

A step names its judge, because plans are data: rule files and inline
strategies can only refer to judges the engine already holds, and
configuration can limit which judges a project's rules may use. A `Panel`
maps names to judges. `plan.run(&panel)` validates the plan, rejects a name
the panel lacks before asking anything, and then asks each judge its share of
every round in one `judge_all` call. A batch with the wrong number of answers
settles as `Failed`. The round-by-round API remains for the playground and the
runtime; each `Question` reports its step, judge and request.

Judges are synchronous so rules stay plain functions. The runtime runs slow
judges such as Jev on bounded worker threads, with deadlines and cancellation;
neither core nor the interface changes for that.

## State machine and event model

`plan.run(&panel)` runs a plan to its end. Underneath, core advances one round
at a time, so a caller may also drive rounds itself: the runtime around slow or
remote judges, or the playground with answers picked by hand. The low-level API
returns events as data:

```rust
let transition = plan.start();
observe(transition.events);
let round = transition.next.ask()?;
let reply = answer_round(round.questions()); // each question names its judge
let transition = round.answer(reply)?;
observe(transition.events);
```

Each transition includes the next state, `Ask { round, questions }` or
`Done { verdict }`, and the events it produced: `Started`, `RoundReady`,
`StepSettled`, `StepSkipped` and `Finished`. Events have stable step IDs,
round numbers and settled outcomes; they contain no transport or
provider-specific types. The only user-supplied code core invokes is a judge,
inside `run`.

`crates/o11y` consumes events for logs, traces or audit. The daemon serializes
each run's events onto its caller's connection from a bounded log; a slow
caller never blocks a core transition. The state machine does not decide
provider choice, retries, HTTP, credentials or actions.

## Playground

The Vite application uses petite-vue for reactivity and Web Awesome for UI
components. Each feature under test is a tab; its Rust implementation is
compiled to WASM.

The **State machine** tab contains:

- A plan editor with an example for each row of the
  [strategies table](#strategies): escalate, rank, collect all that pass,
  check the survivors, and vote, plus a step with both `all` and `any`.
- A validation area for duplicate IDs, `after` naming a later or unknown step,
  and empty `when` lists.
- The steps asked in the current round, with controls to pick each one's
  outcome or failure by hand.
- A transition-event timeline and final per-step verdict.
- Reset/replay controls and a sharable plan fragment in the URL.

The playground invokes the same core state transitions as the native runtime;
it does not contain a second implementation of `after`. The Vite development
loop rebuilds Rust changes in `playground/wasm/` and `crates/`, and reloads
the page. Slice 2 adds the Strategy tab.

## Package layout

The product holds building blocks only. Shipped content lives in
`distribution/` and loads from a path, through the same loader as the user's
own; nothing in `crates/` embeds it.

```mermaid
flowchart LR
    subgraph Product["Product: building blocks, no content"]
        Daemon["chauffeur daemon<br/>loads sources from paths"]
    end
    Dist[("distribution/<br/>models, facts, rules, safety")] -- "shipped, lowest precedence" --> Daemon
    User[("~/.config/chauffeur/")] -- "user" --> Daemon
    Project[(".chauffeur/")] -- "project, limited" --> Daemon
```

```text
crates/
  core/          # Steps, after, rounds, events, verdict. No dependencies
  protocol/      # navigate, rule files, events and actions; validation and limits; lowering
  judges/        # Questions, results, confidence, tiers and fallback; formats:
                 #   systemone, openai, anthropic, person, placeholder, facts
  facts/         # QuickJS sandbox: runs a fact with caps and host modules. Ships no facts
  runtime/       # Admission, deadlines, workers, redaction
  daemon/        # Socket; loads models, facts and rules; event bus; plugin supervisor
  cli/           # chauffeur binary: navigate, strategy show, setup and audit
  o11y/          # Event consumers, on tracing
packages/
  chauffeur/     # @fdcn/chauffeur: /client, /judge, /plugin, /harness, /adapter
adapters/
  opencode/      # The OpenCode adapter
  <host>/        # Each further harness
plugins/
  skills/            # Skill exposure, remote sources and safety review
  context-renewal/   # Compacts long, cold sessions
  warming/           # Keeps the prompt cache warm while idle
  guard/             # Blocks or asks before irreversible actions
distribution/
  models/        # jev.json, subq.json, glide.json
  facts/         # match.js, select.js, bm25.js, embedding.js, and their tests
  rules/         # Shipped rule files
  safety/        # Redaction and backstop patterns
playground/
  wasm/          # Thin wasm-bindgen bridge to core and protocol
  src/features/  # One interactive tab per feature under test
scripts/         # Packaging, Homebrew formula, smoke tests
.github/workflows/
```

| Rule | Consequence |
|---|---|
| `crates/` never embeds content | A build without `distribution/` still runs, with nothing in it |
| One loader for every source | The shipped `select.js` takes exactly the path a user's `notes.js` takes |
| Precedence | Shipped, then user; project content is limited (pure facts, no models, System Two only if allowed) |
| Install | Packaging copies `distribution/` to `~/.local/share/chauffeur/`, so the product and its content release separately |
| Plugins | `plugins/` holds the first-party plugins. Each is its own package on the SDK, registered in `plugins.json` like anyone else's; nothing in `crates/` depends on them |

Crates arrive when the API they consume is reviewed. `o11y` waits until the
core event schema is stable.

## Sequence and verification

### Slice 0: playground shell (implemented)

- Keep the pinned nightly, strict workspace lint setup, LICENSE and plan/audit
  documents.
- Use Vite, TypeScript and Bun; petite-vue provides reactivity and Web Awesome
  supplies tabs, cards and inputs.
- Compile Rust to browser WASM using wasm-bindgen/wasm-pack.
- Prove in a real browser that editing input updates Rust output, and that an
  edit to Rust rebuilds WASM and reloads the page.

### Slice 1: core plan and state machine

Status: to be rewritten to the [core plan model](#core-plan-model), replacing
the gate-based core on the branch. `crates/core` stays dependency-free.

| File | Contents |
|---|---|
| `crates/core/src/plan.rs` | `Plan`, `Step` with `after` and `after_any`, `After`, `Need` (`Need::when`, and a step ID as a need without `when`), `Outcome` |
| `crates/core/src/after.rs` | The one rule: a step's decision from its needs' states (`Wait`, `Run`, `Skip`), and one pass over the plan in order |
| `crates/core/src/state.rs` | Validation, `start`, `Round` (`questions`, `answer`, `fail`), `Plan::run` |
| `crates/core/src/judge.rs` | `Judge`, `Panel` |
| `crates/core/src/event.rs` | Events and `Verdict` |
| `crates/core/src/proofs.rs` | Kani harnesses |
| `crates/core/tests/scenarios.rs` | Scenario tests |
| `playground/wasm/src/state_machine.rs`, `playground/src/features/state-machine.ts`, `playground/index.html` | The State machine tab on the new model |

Scenario tests:

- Each row of the [strategies table](#strategies), with a `select`-like rule
  judge standing in for facts: escalate, rank, collect all that pass, check
  the survivors, vote.
- `after` alone: a string need waits for a skipped step and then runs; an
  object need never matches a skipped step; `Failed` matches only when listed;
  a step with both `all` and `any` runs only when both hold.
- Validation: an empty plan, a duplicate ID, `after` naming a later or unknown
  step, an empty `when` list, and a judge the panel lacks, each rejected
  before any judge is asked.
- The round API: a reply naming an unknown, unasked or repeated step, or
  missing one, returns the unchanged round; each judge answers its share of a
  round in one call; a batch of the wrong length settles as `Failed`.

Kani harnesses, each one invariant over at most three steps and a small
outcome enum:

| Harness | Property |
|---|---|
| `a_step_waits_until_every_need_has_finished` | No decision while a named step is `Pending` or `Asked` |
| `a_decision_follows_the_rule` | `Run` exactly when every `all` need and, if any, one `any` need is met |
| `a_skipped_step_meets_only_needs_without_when` | Skipped never matches a `when` |
| `one_pass_settles_every_decidable_step` | Skips propagate within one pass |
| `a_pass_only_moves_pending_steps_forward` | Each step is asked at most once |
| `a_pass_with_no_open_round_asks_or_finishes` | A run never stalls, so it finishes within one round per step |

Tooling: `bacon.toml` with clippy, test and doc jobs; `.config/nextest.toml`;
`criterion` benchmarks for one pass and a full run of each strategies-table
plan.

Verification: strict Clippy, the scenario tests under nextest and doctests,
Kani, Miri, `cargo-mutants` over `crates/core` with every surviving mutant
killed or explained, `cargo-shear`, and the playground in a real browser, with
each example stepped through by hand. Stop for review before slice 2.

### Slice 2: navigate

A caller sends an event or an inline strategy, with its evidence, candidates
and a deadline, and receives the events and the intents or verdict. It sends
data, never callbacks or code. This slice loads models, facts and rules from
their sources, matches rules by `on` and `when`, and builds `navigate` with
its bindings: the TypeScript client and `/judge`, `chauffeur navigate`, and
the protocol crate.

```mermaid
flowchart LR
    Program["TypeScript program"] --> Client["@fdcn/chauffeur/client<br/>and /judge"]
    Script["Hook script"] -- "stdin" --> Cli["chauffeur navigate"]
    Client -- "JSON-RPC, Unix socket" --> Daemon["crates/daemon"]
    Cli -- "JSON-RPC, Unix socket" --> Daemon
```

#### Wire contract

A `navigate` request, `event` notifications, then the result, as
[JSON-RPC 2.0](#json-rpc-20) lines. `chauffeur navigate` reads the request's
`params` on stdin and prints the messages it receives:

```sh
chauffeur navigate <<'EOF'
{"protocol": 1, "deadline_ms": 1500, "on": "recall", "evidence": {"message": "…"}, "candidates": {"p1": "…", "p4": "…"}}
EOF
# → {"jsonrpc": "2.0", "method": "event", "params": {"id": 1, "rule_matched": {"rule": "opencode-history/recall"}}}
# → {"jsonrpc": "2.0", "method": "event", "params": {"id": 1, "round_ready": {"rule": "opencode-history/recall", "round": 1, "steps": ["helps[p1]", "helps[p4]"]}}}
# → {"jsonrpc": "2.0", "method": "event", "params": {"id": 1, "step_settled": {"step": "helps[p4]", "answer": {"outcome": "yes", "p": {"yes": 0.91, "no": 0.09}, "level": 0.09}}}}
# → {"jsonrpc": "2.0", "method": "event", "params": {"id": 1, "step_settled": {"step": "best", "answer": {"outcome": ["p4"]}}}}
# → {"jsonrpc": "2.0", "id": 1, "result": {"intents": [{"rule": "opencode-history/recall", "attach": ["p4"]}]}}
```

A rejection is a JSON-RPC error whose `data.reason` names it:
`limit_exceeded`, `judge_unavailable`, `busy`, `deadline` or
`unsupported_protocol`.

A request with `strategy` in place of `on` ends with `{"verdict": {…}}`.
`$CHAUFFEUR_SOCKET` defaults to `chauffeur/daemon.sock` in the user's runtime
directory: `$XDG_RUNTIME_DIR`, or `$TMPDIR` on macOS.

- **Answers and failures** are distinct: a step settles to
  `{"answer": {"outcome", "p", "level"}}` or `{"failed": "<reason>"}`.
  Probabilities have at most three decimals.
- **Intents** carry the rule that produced them. A rule's intents are dropped
  if its strategy misses the deadline; other rules' intents still return.
- **Evidence** is named once per navigation and referenced by steps, so a
  batched provider call carries each block once.
- **Versioning:** `protocol` is required; the daemon answers
  `unsupported_protocol` with the versions it accepts, and each binding reports
  it as `unavailable`.
- **Limits**, checked before admission: 32 steps per strategy, counting each
  copy `for_each` makes; 16 `after` entries per step; 64 outcomes per
  question; 64 evidence blocks and candidates; 256 KiB of evidence; 30 s
  deadline. Each rejection names its limit.

#### Files

| Path | Contents |
|---|---|
| `crates/protocol/src/{lib,request,strategy,rule,question,intent,limits,error}.rs` | Serde wire types for requests, strategies, rule files, questions, results and intents; validation and limits; lowering named judges and asks, `for_each` copies, wildcards and `[item]` to `chauffeur_core::Plan`; typed errors |
| `crates/protocol/schema/navigate.json`, `crates/protocol/fixtures/` | The one `navigate` schema every binding uses, and the valid and invalid requests each binding's tests replay |
| `crates/judges/src/{question,confidence,tiers,config}.rs` | Questions and results; applying confidence and computing `level`; resolving a step's `judge` and `model` against shipped and user `models.json` in fallback order, refusing any model entry from a project |
| `crates/judges/src/format.rs` | The `Format` interface and `Failure`; turning a tier's models into one core `Judge` with fallback |
| `crates/judges/src/{facts,placeholder}.rs` | The `facts` format over `crates/facts`, and the `placeholder` format |
| `crates/facts/src/{lib,sandbox,host,limits}.rs` | Loading fact modules from the three sources; QuickJS with memory caps and deadline interrupts; the read-only host modules for shipped and user facts |
| `distribution/facts/{match,select}.js` and their tests | The two facts slice 2 ships |
| `crates/runtime/src/admission.rs` | One permit per navigation, held from admission through delivery; per-source and global capacity |
| `crates/runtime/src/run.rs` | Drives rounds on a bounded worker pool; checks the deadline and cancellation before each round; discards late results |
| `crates/runtime/src/redact.rs` | Redacts evidence before every provider judge, porting v1's rules and their cases |
| `crates/runtime/src/events.rs` | Bounded per-run event log; overflow is an explicit event |
| `crates/daemon/src/{main,socket}.rs` | Creates the socket directory `0700` and refuses one it does not own; JSON-RPC 2.0 lines, several calls per connection, `$/cancelRequest` and closing cancel; `--models FILE` replaces the user's models, for tests |
| `crates/daemon/src/rules.rs` | Loads rule files from the four sources, refuses System Two in a project rule the user has not allowed, reloads rules, facts and models when `notify` reports a change, matches `on` and `when`, and runs matching rules concurrently |
| `crates/cli/src/navigate.rs` | `chauffeur navigate`: one request on stdin, the JSON-RPC messages on stdout; exit 0 with intents or a verdict, 1 when rejected, 2 when the daemon is unavailable |
| `crates/cli/src/strategy.rs` | `chauffeur strategy show <rule>` and `--file`: the strategy as a tree, with rounds, resolved models, `after`, confidence and the maximum rounds and model calls |
| `packages/chauffeur/src/{client,judge}/` | `@fdcn/chauffeur/client` and `@fdcn/chauffeur/judge`: `navigate` over the socket, closing on abort; `ask`, `judge`, `strategy` and the question functions, typed from their questions |
| `playground/wasm`, `playground/src/features/strategy.ts` | Parses strategies through `crates/protocol`; draws a rule's strategy tree and steps through it with recorded answers |

#### Reference caller: opencode-history recall

[Per-turn recall](#per-turn-recall) is the reference caller: its rule
installed in `~/.config/chauffeur/rules/opencode-history/`, and its code
reduced to one `navigate` call. Slice 2 runs it end to end against recorded
scores.

#### Verification

- Protocol: round-trip and rejection cases for every limit and malformed
  field, including duplicate outcomes, reserved outcome names and a
  per-outcome confidence that misses an outcome.
- Runtime under Miri: deadline, cancellation, late result, overflow, and
  exhausted capacity, including audit A3's flood: 81 submissions with capacity
  8 admit 8 and reject 73 with `capacity`.
- Kani: a permit is released exactly once on every terminal path; the event
  log never exceeds its bound; for every probability in thousandths and every
  valid confidence, an answer settles to exactly one outcome, edges inclusive;
  every lowered plan has at most as many rounds as `strategy show` reports.
- Facts: `match` passes v1's facts-judge cases; `select` passes exhaustive
  tests over every input up to 4 candidates; a fact over its memory cap or
  deadline fails the step; a project fact cannot import a host module; a
  missing fact falls back to a placeholder.
- Judges: fallback moves to the next model only when one is unavailable,
  rate-limited or timed out; `"model": "glide"` without GLiDE configured runs
  on the tier's fallback order and reports it; a model entry in a project's
  `.chauffeur/` is refused; a key never appears in an event, error or verdict;
  an outcome outside the question's list fails as `invalid`.
- Strategies: each row of the [strategies table](#strategies) runs as a rule
  with recorded answers; `after` with `[item]` skips exactly the candidates
  whose earlier copy did not match.
- Daemon socket: an unknown judge, an unsupported protocol, an oversized line
  and a closed connection each fail safely; events arrive in order; another
  user's socket directory is refused.
- Rules: each of the four sources loads; a project rule naming System Two is
  refused unless the user allows it; an edited rule applies to the next
  navigation; two matching rules both return their intents; a rule that misses
  the deadline drops only its own intents.
- Placeholders: recorded answers match by evidence, first match wins;
  `otherwise` answers the rest, and without it the step fails as
  `unavailable`; every placeholder answer is marked in events; a placeholder
  from a project's `.chauffeur/` is refused.
- Bindings, against the daemon with placeholder System One and System Two: the
  playground examples return the same verdicts through the client, `/judge`
  and `chauffeur navigate` as through `Plan::run`; every fixture is accepted
  or rejected identically by each; aborting any caller cancels the
  navigation; `ask`'s result types reject impossible outcomes at compile time.
- Recall: Jev scores recorded by opencode-history's replay feed a placeholder
  System One; navigating `recall` attaches the replay's top three passages
  scoring 0.7 or above. No agent session or history is sent.

### Slice 3: System One models (Jev, SubQ and GLiDE)

Implement the `systemone` API format in `crates/judges`, over `ureq`, using
the current `/v1/systemone` contract: authentication, timeout, bounded response, explicit
429/529 handling, `noul` for yes/no questions and `choice` with full
probabilities for the rest, and GLiDE's level-valued `score`. Ship
`jev.json` as the default System One, and `subq.json` and `glide.json` beside
it, in `distribution/models/`. Core and the runtime stay unaware of any model.

Verification: loopback HTTP contract cases for each provider's documented
request/response, invalid replies and provider errors, replaying `main`'s
`judges/{jev,subq,glide}/tests/wire.rs`; then run the Jev-to-GLiDE escalation
from [Strategies](#strategies) against both with real keys, and compare its
answers with `main`'s `CHAUFFEUR_JUDGE_ESCALATE=glide` on the same cases.
Report the strategy, answers, rounds, elapsed time and model-call count. Ask
labelled scale questions through Jev's `choice` and `score` types and report
which agrees with the labels more often. Run opencode-history's recall replay
live through `navigate`: one Jev call per message, judge time within the
replay's 278 ms p50 and 318 ms p95 plus local overhead.

### Slice 4: System Two models

Implement the `openai` and `anthropic` API formats in `crates/judges`. A System
Two answer is schema-checked against the question's outcomes, and its `cites`
must name evidence the step was given. Redaction applies before every call.
Model entries set the model ID, effort, timeout and output budget. System Two
ships disabled until a model and its evaluations are reviewed.

Verification: loopback cases for valid, refused, malformed, truncated,
unavailable and timed-out replies; screen-then-review round ordering;
citations of evidence the step lacks are rejected; the 32 KiB input budget;
and proof that a slow reasoning call cannot consume reserved interactive
System One capacity. Then replay human-labelled cases before enabling a live
model.

### Slice 5: plugins and adapters

Build the [plugin](#plugins) and [adapter](#adapters) halves: the daemon's
event bus and plugin supervisor, the stdio plugin protocol with discovery,
lazy and eager start, `before` budgets and `onTimeout`, retries by kind, and
`@fdcn/chauffeur/plugin`, `/harness` and `/adapter`. Build the OpenCode
adapter over the full OpenCode V2 plugin API, with gdp-ts proofs before every
action. Then port each parity-table row assigned to slice 5 as a plugin or a
rule file in `distribution/rules/`.

v1's event gate becomes `sourcefed/triage`. In sourcefed, its daemon navigates
`integration_event` before queuing each event, its installer adds the rule,
and its OpenCode adapter's `chauffeur.gate` call goes. Before the slice closes,
review every `main` commit past the watermark and add its row.

| File | Contents |
|---|---|
| `crates/protocol/schema/{events,actions}.json` | The standard events and the resource and verb actions |
| `crates/daemon/src/{bus,plugins,adapters}.rs` | Routing events to rules and plugins; supervising plugins (discovery cache, lazy start, idle stop, backoff); adapter connections and their declared actions |
| `packages/chauffeur/src/{plugin,harness,adapter}/` | The plugin, action and adapter SDKs |
| `adapters/opencode/` | The OpenCode adapter, and `navigate` over plugin RPC for other OpenCode plugins |
| `plugins/{skills,context-renewal,warming,guard}/` | The first-party plugins, each its own package on the SDK |

A spike decides whether first-party plugins ship compiled with scriptc. It
compiles `context-renewal` and `guard`, and reports which SDK and Node APIs
scriptc rejects, start time and memory against Bun, and build time. Compiled
plugins are rebuilt with each release; user plugins, and first-party plugins
in development, keep running from source on Bun so a file change reloads
them.

Verification: each row's evidence from `main` passes against v2; every v1 rule
converts and validates; a plugin crash, a missed `before` budget and a stale
action each behave as specified; `onTimeout: "ask"` hands a tool call to the
user; discovery records the same hooks a full start registers; an unsupported
action returns `unsupported`; sourcefed events reaching OpenCode, Pi and the
CLI are filtered alike, and v1's 12 labelled events give the same deliveries;
the bench `verify` fixtures pass; a day of real use through a locally built
adapter, plugins and daemon.

### Slice 6: release and cutover

Restore `check.yml`, `scripts/package.mjs`, `scripts/homebrew.mjs` and
`scripts/smoke.mjs` for the v2 layout, packaging `distribution/` beside the
binary. Ship `1.0.0-rc.N` to the `next` channel; the handshake rejects a v1
daemon, plugin or adapter. Migrate `~/.config/chauffeur`, or read v1's layout,
without losing personal rules. When every parity row is done, keep `v1` as a
hotfix branch, merge v2 into `main` and tag `1.0.0`.

Verification: packaging dry run and smoke test on Linux and macOS; Homebrew
`install`, `test`, `audit` and `style`; upgrade from the current v1 release on
a machine with personal rules; rc release-gate runs from the bench.

## Quality gates

- Pinned nightly, deny-level strict Clippy, formatting and stable MSRV check.
- Miri over core and runtime scenarios; Kani proofs are small and each names
  one invariant. `crates/facts` is outside Miri because QuickJS is C; its
  tests run natively.
- TypeScript: strict `tsc`, the gdp-ts lint preset, and type tests that
  impossible outcomes fail to compile.
- Real boundary checks for browser WASM, the daemon socket, each `navigate`
  binding, the plugin protocol, provider HTTP and live Jev. Unit tests
  supplement these checks; they do not replace them.
- Verify at each slice before proceeding. Keep functions under 70 non-blank,
  non-comment lines, use no production dependency without approval, and keep
  core dependency-free.
- Tests run under `cargo nextest`, with doctests through `cargo test --doc`.
  `cargo-mutants` runs over core, confidence and lowering, and every
  surviving mutant is either killed by a new test or explained. `cargo-shear`
  finds no unused dependency. `criterion` benchmarks back every latency the
  plan states.
- Full workspace tests, Clippy, Miri, Kani, docs and deny run before committing
  the completed slice. The v2 branch is pushed after the agreed review.

## Decisions

| Concern | Decision |
|---|---|
| Wire protocol | JSON-RPC 2.0 over the Unix socket and stdio |
| First-party plugins | `plugins/` in this repository, each its own package on the SDK |
| Jev's `score` type | `choice` for every question that is not yes/no; slice 3 measures `score` on labelled scale cases, and a question gains a hint only if `score` is clearly better |
| Production dependencies | Core stays dependency-free. Approved: `serde` and `serde_json` (protocol and every config file), `thiserror` (typed errors outside core), `rquickjs` and bundled `rusqlite` (facts and the `chauffeur:sqlite` host module), `notify` (reloading rules, facts and models), `ureq` with rustls (model formats, slices 3 and 4), `tracing` and `jiff` (o11y events and audit timestamps), `clap` and `color-eyre` (the CLI binary only) |
| Development tools | `cargo-nextest` runs tests (doctests stay on `cargo test --doc`); `cargo-mutants` checks that tests catch changed logic in core, confidence and lowering; `cargo-shear` catches unused dependencies; `criterion` benchmarks the socket hop, confidence, a fact call and lowering; `bacon.toml` holds shared watch jobs |
| scriptc | A slice 5 spike for first-party plugins; the engine stays Rust |
