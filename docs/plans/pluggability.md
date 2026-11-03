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
  events and carry out actions. They come from the shared harness package,
  which any program can use; Chauffeur is one of the deciders it asks.
- **Callers**, any program, send a question with its context through
  `chauffeur.navigate`, with or without a harness. Rule files, which are
  data, decide how to answer. [Integrations](../integrations/README.md)
  has one page per program.

Shipped content (models, facts, rules) lives apart from the engine, in
`distribution/`, and loads through the same path as the user's own.

Each slice delivers one feature a person uses, working through the real path
and installable, and builds only the pieces that feature needs. The next
slice's feature brings the next pieces. See
[Slices](slices/README.md).

| #   | Feature                                                          | New pieces                                                                                |
| --- | ---------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| 0   | [Setup](slices/slice-0.md)                                       | Toolchain, lints, the playground shell                                                    |
| 1   | [Conventional Commits nudge in OpenCode](slices/slice-1.md)      | Core, rule files, the Jev format, the daemon as a decider, rc release                     |
| 2   | [sourcefed triage](slices/slice-2.md)                            | `navigate` for programs, intents, the TypeScript client                                   |
| 3   | [Escalation from Jev to GLiDE](slices/slice-3.md)                | Multi-step strategies with `after`, SubQ and GLiDE, `choice` and `score`, `strategy show` |
| 4   | [opencode-history recall](slices/slice-4.md)                     | Facts, `for_each`, `select`                                                               |
| 5   | [Harm guard, permissions and v1's rules](slices/slice-5.md)      | Plugins, `ctx.before`, asking the user, the rule-file fields v1's rules need              |
| 6   | [Skills and tools](slices/slice-6.md)                            | The skills, tools and `ask-chauffeur` plugins, prompt edits                               |
| 7   | [Context renewal, warming and model failover](slices/slice-7.md) | Session actions                                                                           |
| 8   | [Pi](slices/slice-8.md)                                          | Every feature so far, through the harness package's Pi adapter                            |
| 9   | [Claude Code](slices/slice-9.md)                                 | The same, through its Claude Code adapter                                                 |
| 10  | [Codex](slices/slice-10.md)                                      | The same, through its Codex hook command                                                  |
| 11  | [Review by a reasoning model](slices/slice-11.md)                | The System Two formats                                                                    |
| 12  | [Rulebooks](slices/slice-12.md)                                  | Held until discussed                                                                      |
| 13  | [Cutover](slices/slice-13.md)                                    | v2 replaces v1                                                                            |

The work proceeds one slice at a time, with review before advancing.

```mermaid
flowchart LR
    S0["0 Setup"] --> S1["1 Commit nudge<br/>OpenCode, Jev"]
    S1 --> S2["2 sourcefed triage<br/>navigate"]
    S2 --> S3["3 Escalation<br/>after, GLiDE"]
    S3 --> S4["4 Recall<br/>facts, select"]
    S4 --> S5["5 Guard<br/>plugins"]
    S5 --> S6["6 Skills, tools"]
    S6 --> S7["7 Renewal, warming,<br/>failover"]
    S7 --> S8["8–10 Pi, Claude Code, Codex"]
    S8 --> S11["11 System Two"]
    S11 -.-> S12["12 Rulebooks<br/>held"]
    S12 --> S13["13 Cutover"]
```

## Production and parity with main

Users run v1, released from `main`. v2 changes nothing they install until
cutover, and `main` keeps receiving features and fixes meanwhile.

```mermaid
flowchart LR
    Main["main (v1)<br/>features and fixes"] --> CI["check.yml<br/>fmt, clippy, tests, package smoke"]
    CI --> Brew["Homebrew release<br/>what users run"]
    Main -. "ported by behavior<br/>tracked below" .-> V2["v2 branch<br/>one feature per slice"]
    V2 --> RC["1.0.0-rc.N<br/>next channel"]
    RC --> Cut["Cutover<br/>v2 merged into main"]
```

| Concern        | Decision                                                                                                                                                                                  |
| -------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Porting        | Each `main` feature is re-expressed in v2's architecture, not merged: v2's tree replaces v1's, so merges would conflict on every removed file                                             |
| Tracking       | The parity table lists every `main` commit after the branch point; the watermark names the last one reviewed                                                                              |
| Evidence       | A ported feature passes `main`'s cases for it, replayed against v2                                                                                                                        |
| Version mixing | The version handshake between the daemon, its SDK, plugins and the harness package keeps v1 and v2 from running together                                                                  |
| User state     | v2 reads no v1 file. At cutover an agent rewrites the user's configuration into v2's files, from the prompt in the [migration guide](../migration/README.md#rewriting-your-configuration) |
| After cutover  | `v1` is a branch for hotfixes only                                                                                                                                                        |

### Parity table

The [migration guide](../migration/README.md) shows every v1 feature's file
beside the v2 rule file or plugin that replaces it.

Branch point `73bc29e`. Watermark: `c3b4e1b`.

| `main` commit                                                                  | v2 home                                                                                                                                                                                                                                                                                                                               | Slice    | Evidence from `main`                                                                             |
| ------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------- | ------------------------------------------------------------------------------------------------ |
| `f2b087b` judge interface, facts judge                                         | The `match` fact in `distribution/facts/`; `judge.json` routes become step judge names                                                                                                                                                                                                                                                | 4        | `judges/facts/src/lib.rs` tests                                                                  |
| `bb41c9a` context renewal, session warming                                     | The `context-renewal` and `warming` plugins; the adapter carries out `session.compact` and `session.warm`                                                                                                                                                                                                                             | 7        | `routing.rs`, `renewal.test.ts`, `warming.test.ts`                                               |
| `22e5391`, `36f6253` skill safety review                                       | The skills plugin's screen-then-review strategy                                                                                                                                                                                                                                                                                       | 6        | `sources/tests.rs` vetting cases                                                                 |
| `3a18675` Skilld discovery                                                     | The harness's own skill sources: the skills plugin reads the harness's catalog, such as OpenCode's `skills` paths and HTTP catalogs, and fetches nothing                                                                                                                                                                              | 6        | `cli/tests/skills.rs` discovery cases, against an HTTP catalog in OpenCode's config              |
| `c43ecd6` read guard, RPC starvation fix                                       | The guard plugin's `before` hooks; runtime admission permits (audit A3)                                                                                                                                                                                                                                                               | 2, 5     | `read-guard-probes.json`, `rpc-starvation.test.ts`                                               |
| `9b8e917` rules                                                                | Rule files in `distribution/rules/`; each v1 rule lands in the first slice that has the pieces it needs                                                                                                                                                                                                                               | 1–11     | `capabilities/rules/tests/rules.rs`                                                              |
| Rulebooks (`/goal`, `/ticket`)                                                 | Rulebook files run by the `rulebooks` plugin; see [Rulebooks](#rulebooks)                                                                                                                                                                                                                                                             | 12       | `skills/rulebooks/*.json`, `rulebooks.test.ts`                                                   |
| `73bc29e` Homebrew release, CI                                                 | Release scripts, `check.yml` and `release.yml` in v2: rc releases as `chauffeur-next` from slice 1, `1.0.0` at cutover                                                                                                                                                                                                                | 1, 13    | `release.test.mjs`, `smoke.mjs`                                                                  |
| `f54db24`, `d66a385`, `e7fbabf` docs                                           | Documentation only                                                                                                                                                                                                                                                                                                                    | —        | —                                                                                                |
| `5330c3b` GLiDE, `fab9ae1` no escalation for permission requests, who answered | `glide.json` over the `systemone` format; escalation is a strategy: System One with `model: "jev"`, then `model: "glide"`, then System Two. The guard's strategy asks one judge and never escalates, since the agent's call waits on it; each round's event names the model that answered and keeps the answers a later step replaced | 3, 5, 11 | `judges/glide/tests/wire.rs`, `docs/judges.md` escalation cases                                  |
| `0adc459` a failed permission check keeps the host's decision                  | Rules on `permission.asked`: a failed or late answer keeps what the harness decided, since Chauffeur only vetoes what the host allows; a 5 s `before` budget; the harness package retries an unreachable daemon twice, 500 ms apart                                                                                                   | 5        | `permission.test.ts`                                                                             |
| `16c1433`, `1949263`, `413eb65` tools that stay exposed                        | The `tools` plugin's `base` list: `history_search`, the agent conversation tools and the `agents` Code Mode namespace                                                                                                                                                                                                                 | 6        | `tool-exposure/tests/exposure.rs`, `skills/config/tool-exposure.json`                            |
| `94e3b4e` 32 remote skill sources                                              | The harness's config sets the sources; Chauffeur keeps no list or limit of its own                                                                                                                                                                                                                                                    | 6        | —                                                                                                |
| `02aee18`, `60d4c26` renewal fixes                                             | The context-renewal plugin writes a summary in a temporary child session on the summary model, passes over a provider that failed for an hour, and leaves a session already on a cheap model to the harness's own compaction                                                                                                          | 7        | `renewal.test.ts`, `model-router/tests/routing.rs`                                               |
| `c3b4e1b` reworded questions                                                   | The wording ships in each rule's `instructions`: the shell file guards (5), `cat-to-read` and `hand-rolled-patch` (the first slice with their pieces) and the Code Mode namespace question (6), each replayed against `main`'s recorded requests                                                                                      | 1–6      | `skills/permission/shell-file-*-guard.json`, `skills/rules/{cat-to-read,hand-rolled-patch}.json` |
| Uncommitted on `main`                                                          | Reviewed once committed                                                                                                                                                                                                                                                                                                               | —        | —                                                                                                |

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
        Harness["/harness"]
    end
    Adapters["Harness package<br/>one adapter per harness"]
    subgraph Engine["Rust engine"]
        Daemon["daemon<br/>socket, event bus, plugin supervisor"]
        Run["protocol, runtime, judges, facts"]
        Core["core<br/>after, rounds, events"]
    end
    Dist[("distribution/<br/>models, facts, rules")]
    TS -- "Unix socket or stdio" --> Daemon
    Adapters -- "events and before, as a decider" --> Daemon
    Daemon -- "actions" --> Adapters
    Daemon --> Run
    Run --> Core
    Dist --> Daemon
```

| Kind      | What it is                                                      | Written by                                       | Examples                                                                  |
| --------- | --------------------------------------------------------------- | ------------------------------------------------ | ------------------------------------------------------------------------- |
| Rule file | Data: `on`, `when`, `judges`, `asks`, `strategy`, `then`        | Chauffeur, an integration, the user or a project | `conventional-commits`, `sourcefed/triage`, `opencode-history/recall`     |
| Fact      | A JavaScript module that answers a question without a model     | Chauffeur, the user, or a project (pure only)    | `match`, `select`, `bm25`, a user's `notes`                               |
| Plugin    | A TypeScript program that hooks events, asks questions and acts | Chauffeur, the user, anyone                      | `skills`, `context-renewal`, `warming`, `guard`                           |
| Caller    | Any program that sends a question and its context               | Anyone                                           | sourcefed's daemon, opencode-history, hook scripts                        |
| Adapter   | Per-harness glue: standard events out, actions in               | The shared harness package, one per harness      | Its OpenCode plugin, Pi extension, Claude Code mod and Codex hook command |

Policy lives in rule files, facts and plugins. Adapters and callers hold no
judging logic: an adapter translates, and a caller sends its question and acts
on the intents it gets back.

### Dependency direction

```mermaid
flowchart TB
    Core["crates/core<br/>Generic plan state machine"]
    O11y["crates/o11y<br/>Observes core events"] --> Core
    Protocol["crates/protocol<br/>Requests, rules, events, actions"] --> Core
    Protocol --> Judges
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

|                                                 | Result                     |
| ----------------------------------------------- | -------------------------- |
| One message over the Unix socket, 20 KB of JSON | 0.023 ms p50, 0.232 ms p99 |
| One Jev call (opencode-history's recall replay) | 278 ms p50                 |
| v1's Rust daemon at rest                        | 66 MB                      |
| Idle Bun, idle Node                             | 10 MB, 37 MB               |

| Concern      | Rust's part                                                                                                                                                                                                                                                              |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Safety       | The deciding reason. The daemon parses untrusted input (the socket, project rules, project facts), holds model keys and redacts evidence. Rust gives memory safety there, `serde` rejects malformed input, and Kani and Miri prove strategies finish within their limits |
| Speed        | Not decisive: a hop costs 0.03 ms against a 278 ms model call                                                                                                                                                                                                            |
| Resource use | Decided by design more than language: lazy plugins, idle shutdown and bounded caches                                                                                                                                                                                     |

Plugins, adapters and the SDK are TypeScript. gdp-ts types the places where
a check must come before an action (see [Proofs](#proofs)).

## `chauffeur.navigate`

Callers use one operation, `chauffeur.navigate`: the decision a chauffeur
makes before steering. A request names an event and carries its context;
every rule whose `on` and `when` match answers it, and the caller receives
their intents:

```ts
import { navigate } from "@fdcn/chauffeur/client"

const { intents } = await navigate(
  {
    on: "opencode-history.recall",
    evidence: { message },
    candidates: { p1: "…", p4: "…", p7: "…" }
  },
  { signal: AbortSignal.timeout(1_500) }
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
- **One call, one connection.** `navigate` connects to `$CHAUFFEUR_SOCKET`,
  sends the request, and closes the connection when the result arrives or
  the signal aborts, so a script exits as soon as it is done. A program that
  makes many calls keeps one connection with `connect()`, which reconnects
  after a daemon restart, and closes it itself:

  ```ts
  const chauffeur = connect() // connect({ socket }) for another daemon
  const triage = await chauffeur.navigate(request, { signal })
  chauffeur.close()
  ```

  Inside a plugin, both go over the plugin's stdio, which the daemon owns, and
  `close` does nothing.

### sourcefed: one triage for every host

sourcefed's daemon navigates `sourcefed.event` before it queues an event for
a target, so OpenCode, Pi, MCP and CLI all receive the same filtered stream.
sourcefed installs `sourcefed/triage`, a System One screen that withholds
confident noise; the user tunes or extends it in JSON.

```mermaid
sequenceDiagram
    participant S as sourcefed daemon
    participant C as Chauffeur daemon
    participant H as Any host (OpenCode, Pi, MCP, CLI)
    S->>C: navigate sourcefed.event, with event, monitor and thread
    C->>C: Run sourcefed/triage and any other matching rule
    C-->>S: withhold, or no intent
    S->>H: Queue the event unless withheld
```

With no Chauffeur, an error, or no answer within 3 seconds, sourcefed
delivers, as v1's gate does.

### opencode-history: recall

opencode-history searches its own index when the user sends a message, then
navigates `opencode-history.recall` with the message and the passages as candidates. Its rule,
`opencode-history/recall`, ranks them, and opencode-history attaches the
passages the `attach` intent names.

### Programs without a harness

A program needs no harness to ask Chauffeur. neo navigates
`neo.screen-check` with an iPhone screenshot before each irreversible tap,
and jev-distill's students answer as System One models. Each has
a page in [Integrations](../integrations/README.md).

### One protocol, many bindings

`navigate` is a Chauffeur operation, not any host's. Its request and result
are defined once, in `crates/protocol/schema/navigate.json`, and every binding
forwards them to the daemon socket unchanged:

| Binding                  | Caller                                          | Mechanism                                             |
| ------------------------ | ----------------------------------------------- | ----------------------------------------------------- |
| `@fdcn/chauffeur/client` | Any TypeScript program                          | `navigate(request)` over the socket                   |
| `chauffeur navigate`     | Hook scripts, Git hooks, CI on the same machine | One request on stdin; the JSON-RPC messages on stdout |
| `crates/protocol`        | Rust programs                                   | The same types the daemon uses                        |

Adding a host or a language adds a binding, never a new request format. The
schema's fixtures run in the protocol crate's tests and in each binding's
tests.

### JSON-RPC 2.0

Every message to and from the engine is JSON-RPC 2.0, one message per line,
over two transports:

| Transport       | Used by                    | Why                                                                                        |
| --------------- | -------------------------- | ------------------------------------------------------------------------------------------ |
| The Unix socket | Clients, the CLI, adapters | Independent programs reach the daemon; the socket's `0700` directory is the access control |
| stdio           | Plugins the daemon starts  | A plugin's lifetime is tied to the daemon's, with no socket permissions to manage          |

The same messages travel on both. A request with an `id` gets a result or an
error; a message without one is a notification, used for streamed events.
`$/cancelRequest` with a request's `id` cancels it, and closing a connection
cancels everything it carried. JSON-RPC is the standard LSP and MCP use: it
needs nothing beyond `serde_json`, and its messages stay readable when
debugging.

| Method            | Direction        | Carries                                                                   |
| ----------------- | ---------------- | ------------------------------------------------------------------------- |
| `navigate`        | Caller → daemon  | A request; `event` notifications stream back, then the intents or verdict |
| `emit`, `before`  | Adapter → daemon | A harness event; `before` returns the edits plugins made                  |
| `act`             | Daemon → adapter | An action, with the revision it applies to                                |
| `start`           | Daemon → plugin  | Discovery or run mode, options and the actions available                  |
| `hooks`           | Plugin → daemon  | The events the plugin hooks with `on` and `before`                        |
| `event`, `before` | Daemon → plugin  | A hooked event; `before` returns edits                                    |
| `act`, `navigate` | Plugin → daemon  | An action, or a question for the engine                                   |

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

| In a rule                                                                  | Answered by                                                                                                     |
| -------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| `"quick": { "tier": "system-one", "confidence": 0.7 }`                     | The tier's models, in the user's fallback order                                                                 |
| `"careful": { "tier": "system-one", "model": "glide", "confidence": 0.7 }` | GLiDE; the tier's fallback order applies if GLiDE is unavailable or not configured, and the run's events say so |
| A step with `"judge": "facts"`                                             | The facts tier, with no model and no confidence                                                                 |

Naming the model lets one strategy use two models of the same tier, such as
Jev for a fast screen and GLiDE, more accurate but slower, for what Jev leaves
unsure. A rule shared with someone who lacks GLiDE still runs, on their
System One.

### Questions

A question is instructions and a list of possible outcomes. Nothing else
reaches a judge:

```jsonc
{
  "instructions": "What should the agent do with this message?",
  "outcomes": [
    "act",
    "ignore",
    { "value": "ask-user", "description": "Unclear; the user decides" }
  ],
  "evidence": ["thread", "event"]
}
```

- 2 to 64 outcomes, each unique. An outcome is a string, or a `value` with a
  `description` that the model sees.
- The list's order is meaningful: it is the order of a scale, when the
  outcomes are one.
- `unsure` and `failed` are reserved and cannot be outcomes.
- `evidence` names the request's evidence blocks the judge sees. A block is
  text, or an image (bytes and a media type, at most 4 MiB). Only formats that
  accept images receive one, such as Jev with its opt-in; an image is never
  redacted, so only a step that names it sends it.

The SDK's question functions build this shape, using Jev's names:

| Function                               | Builds                                                               | Example                                                                            |
| -------------------------------------- | -------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| `noul(instructions)`                   | `outcomes: ["yes", "no"]`                                            | "Does this commit message break Conventional Commits?"                             |
| `choice(outcomes, instructions)`       | `outcomes` as given                                                  | "What should the agent do with this message?"                                      |
| `score(levels, instructions)`          | `outcomes` as given, lowest first                                    | "How hard is the user's request?"                                                  |
| `question({ instructions, outcomes })` | The primitive the others use; anyone can write new functions over it | `question({ instructions, outcomes: ["none", "minor", "major", "irreversible"] })` |

### Results

A judge either answers or fails. Failure is not an outcome:

| Side    | Carries                                                                                                                                                                            |
| ------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Answer  | `outcome`: one of the question's outcomes, or `unsure`. `p`: a probability for every outcome. `level`: the probability-weighted position in the list, from 0 (first) to n−1 (last) |
| Failure | Why: `unavailable`, `rate_limited`, `timeout`, `invalid` or `deadline`                                                                                                             |

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

| `confidence`                 | Outcome                                                                                                                                                      |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `0.7`                        | The likeliest outcome when its probability is 0.7 or more; otherwise `unsure`                                                                                |
| `{ "yes": 0.9, "no": 0.95 }` | Per outcome: here `yes` needs 0.9 and `no` needs 0.95. The map must cover every outcome of every question the judge is used for, checked when the rule loads |
| none                         | The likeliest outcome; a tie is `unsure`                                                                                                                     |

```text
confidence 0.7 on noul "Does this commit message break Conventional Commits?"
  "fixed stuff"              yes 0.94, no 0.06 → yes
  "feat(cli): add navigate"  yes 0.06, no 0.94 → no
  "Update parser.rs"         yes 0.58, no 0.42 → unsure
  Jev timed out                                → failed: timeout
```

```text
confidence { yes: 0.9, no: 0.95 } on noul "Could this command cause irreversible harm?"
  "rm -rf ~/Projects"  yes 0.96, no 0.04 → yes
  "ls -la"             yes 0.02, no 0.98 → no
  "git clean -fd"      yes 0.12, no 0.88 → unsure, so the user is asked
```

A failure stays a failure whatever the confidence, so it never reads as `no`.
The runtime applies confidence the same way for every tier, in thousandths so
comparisons stay exact, and models never see it. Each judge's confidence is
tuned against labelled cases.

### Tiers

| Tier         | Answers with                                                            | Shipped                                |
| ------------ | ----------------------------------------------------------------------- | -------------------------------------- |
| `facts`      | JavaScript functions in a sandbox: no model and no credentials          | `match`, `select`, `bm25`, `embedding` |
| `system-one` | A fast classifier, one batched call per round                           | `jev` (default), `subq`, `glide`       |
| `system-two` | A reasoning model, with a schema-checked answer that cites its evidence | Disabled until reviewed (slice 11)     |
| `person`     | The user, asked through the adapter                                     | Each adapter                           |

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

| Layer       | Defined in                                 | Example                                                                          |
| ----------- | ------------------------------------------ | -------------------------------------------------------------------------------- |
| Named judge | A rule's `judges`, or `judge()` in the SDK | `"quick": { "tier": "system-one", "confidence": 0.7 }`                           |
| Tier        | `models.json` `tiers`                      | `"system-one": ["jev", "subq"]`                                                  |
| Model entry | `models.json` `models`                     | `"jev": { "format": "systemone", "endpoint": "…", "key": { "env": "JEV_KEY" } }` |
| Format      | Rust in `crates/judges`                    | `systemone`, `openai`, `anthropic`, `facts`, `person`, `placeholder`             |

Every format implements `answer_all`. A model answers every question it
received in a round with one call, and the format decides how to ask its
model from the question's outcomes:

```rust
let replies = jev.answer_all(&round_questions, &evidence);
// → [Ok({ yes: 0.94, no: 0.06 }), Err(Failure::Timeout), …]
```

| `Failure`                               | Then                          |
| --------------------------------------- | ----------------------------- |
| `Unavailable`, `RateLimited`, `Timeout` | The tier's next model answers |
| `Invalid`                               | The step fails                |

| Format        | Speaks                                                                                                                                                                                                                                |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `systemone`   | The `/v1/systemone` API that Jev, SubQ Snap and Fastino GLiDE share: its `noul` type for `["yes", "no"]`, and `choice`, which returns every outcome's probability, for the rest. GLiDE's entry sets its reading of `score` as a level |
| `openai`      | OpenAI-compatible chat completions, with a JSON schema whose `enum` is the outcomes; covers local servers such as Ollama                                                                                                              |
| `anthropic`   | Anthropic Messages with the same schema                                                                                                                                                                                               |
| `facts`       | Runs a fact in `crates/facts`                                                                                                                                                                                                         |
| `person`      | A question to the user through the adapter                                                                                                                                                                                            |
| `student`     | A jev-distill student over its loopback server: the task's labels are the outcomes, and evidence blocks fill its `state`                                                                                                              |
| `placeholder` | Recorded or fixed answers, standing in until the real model is configured                                                                                                                                                             |

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
    "local-qwen": {
      "format": "openai",
      "endpoint": "http://127.0.0.1:11434/v1",
      "model": "qwen3:4b"
    },
    "reviewer": {
      "format": "anthropic",
      "model": "claude-sonnet-5-5",
      "key": { "env": "ANTHROPIC_API_KEY" },
      "timeout_ms": 20000
    }
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
    "one-stand-in": {
      "format": "placeholder",
      "answers": "fixtures/triage-answers.json"
    },
    "two-stand-in": {
      "format": "placeholder",
      "otherwise": { "act": 0.8, "ignore": 0.2 }
    },
    "person-stand-in": {
      "format": "placeholder",
      "otherwise": { "yes": 1, "no": 0 }
    }
  },
  "tiers": {
    "system-one": ["one-stand-in"],
    "system-two": ["two-stand-in"],
    "person": ["person-stand-in"]
  }
}
```

```jsonc
// fixtures/triage-answers.json
{
  "answers": [
    {
      "evidence": { "event": "thanks, looks good 👍" },
      "p": { "yes": 0.52, "no": 0.48 }
    },
    {
      "evidence": { "event": "@agent bump retries to 5" },
      "p": { "yes": 0.94, "no": 0.06 }
    },
    { "evidence": { "event": "deploy is on fire" }, "failed": "timeout" }
  ]
}
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
    subgraph Stand["A placeholder answers for each tier until its slice"]
        F["facts"]
        One["system-one"]
        Two["system-two"]
        P["person"]
    end
    One -- "slice 1" --> Jev["jev"]
    One -- "slice 3" --> More["subq, glide"]
    F -- "slice 4" --> Facts["match, select, your facts"]
    P -- "slice 5" --> Host["adapter asks the user"]
    Two -- "slice 11" --> Rev["openai, anthropic"]
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
  const rows = query(
    "~/notes/index.db",
    "SELECT id FROM notes WHERE notes MATCH ? ORDER BY bm25(notes) LIMIT ?",
    [evidence.message, args.top ?? 10]
  )
  return { ranked: rows.map((row) => row.id) }
}

// "notes": is the message about something already in the notes?
export default function ({ evidence }) {
  const [hit] = query(
    "~/notes/index.db",
    "SELECT 1 FROM notes WHERE notes MATCH ? LIMIT 1",
    [evidence.message]
  )
  return { p: { yes: hit ? 1 : 0, no: hit ? 0 : 1 } }
}
```

A facts step names the fact and its arguments as its `ask`:

```jsonc
{ "id": "related", "judge": "facts", "ask": { "notes.bm25": { "top": 10 } } }
{ "id": "is-slack", "judge": "facts", "ask": { "match": { "field": "event.source", "equals": "slack" } } }
```

| A fact receives                                         | A fact returns                                   |
| ------------------------------------------------------- | ------------------------------------------------ |
| `evidence`, the request's named blocks                  | `{ p }` over the outcomes, usually 1 and 0       |
| `candidates`                                            | `{ ranked: [...] }`: candidate names, best first |
| `outcomes` of earlier steps, with their `p` and `level` | Throws: the step fails as `invalid`              |
| `args`, from the step                                   |                                                  |

| Source  | Folder                       | Host modules                                                                            |
| ------- | ---------------------------- | --------------------------------------------------------------------------------------- |
| Shipped | `distribution/facts/`        | `chauffeur:fs` (read), `chauffeur:sqlite` (read-only), `chauffeur:http` (loopback only) |
| User    | `~/.config/chauffeur/facts/` | The same                                                                                |
| Project | `.chauffeur/facts/`          | None: pure functions of their inputs                                                    |

### Shipped facts

| Fact        | Does                                                                                                                                                                                                       |
| ----------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `match`     | Conditions on evidence fields: `equals`, `one_of`, `contains`, `pattern` (glob), `at_least`, `at_most` (a number, or another field times `times`); combined with `all` and `any`; `cases` answers a choice |
| `select`    | Combines earlier outcomes: `first` settled in a list, `top` N, all at or above `min`, `majority` or `mean`; `by` names the outcome whose probability ranks, such as `yes`                                  |
| `bm25`      | Ranks candidates against a query                                                                                                                                                                           |
| `embedding` | Ranks candidates by similarity from the local embedding service                                                                                                                                            |

v1's context-renewal rule, as a `match`:

```jsonc
{
  "id": "renew",
  "judge": "facts",
  "ask": {
    "match": {
      "all": [
        { "field": "idle_seconds", "at_least": { "field": "cache_seconds" } },
        {
          "field": "context_tokens",
          "at_least": { "field": "context_window", "times": 0.5 }
        }
      ]
    }
  }
}
```

A field the evidence lacks fails the step as `invalid`, never `no`.

### Limits

| Concern        | Decision                                                                                                                                                                                                          |
| -------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Runaway code   | Each call has a memory cap (16 MiB by default) and an interrupt at the step's deadline; overrunning either fails the step                                                                                         |
| A missing fact | `unavailable`, so the facts tier's next model answers: a placeholder can stand in                                                                                                                                 |
| Names          | A fact cannot be named `instructions`, `outcomes` or `evidence`                                                                                                                                                   |
| Project facts  | Run with no host modules. QuickJS is not a hardened sandbox, so a memory-safety bug in it could let a repository's code escape; the caps and the absence of host modules keep that risk small, and it is accepted |
| Engine         | `rquickjs` in `crates/facts`; core stays dependency-free                                                                                                                                                          |

## Strategies

A strategy is a list of steps, built from four pieces. Chauffeur has no list
of built-in strategies: any combination of the pieces is one.

| Piece            | What it does                                                                 |
| ---------------- | ---------------------------------------------------------------------------- |
| Step             | Asks one judge one question; its confidence turns the answer into an outcome |
| `after`          | Waits for earlier steps, then runs the step or skips it by their outcomes    |
| `for_each`       | Copies a step once per candidate, in one round                               |
| Earlier outcomes | A step may read earlier steps' outcomes, `p` and `level` as its evidence     |

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

| Strategy              | Built from                                                                              |
| --------------------- | --------------------------------------------------------------------------------------- |
| Escalate              | A step `after` an earlier one, `when` it is `unsure` or `failed`                        |
| Rank                  | A `for_each` step, then `select` with `top` and `min`                                   |
| Collect all that pass | A `for_each` step, then `select` with `min`                                             |
| Check the survivors   | Two `for_each` steps, the second `after` `first[item]` `when` `yes`                     |
| Vote                  | Parallel steps, `select` with `majority`, and a review `after` the vote `when` `unsure` |

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

| Field                                  | Meaning                                                                                                                                                            |
| -------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `on`, `when`                           | The event that triggers the rule, and a cheap filter on it, such as tool names and text the input contains; no judge is asked unless it matches                    |
| `judges`                               | Named judges: a tier, an optional model and a confidence                                                                                                           |
| `asks`                                 | Named questions, so steps that ask the same question share it                                                                                                      |
| `strategy`                             | The steps, in order: each with an `id`, a `judge` (a named judge or a tier), an `ask` (a named question, one written inline, or a fact), and `after`               |
| `then`                                 | Intents by outcome, first match wins; no match returns no intent                                                                                                   |
| `priority`, `once`, `cooldown_seconds` | Which rule's message wins when one event triggers more than two; firing once until the next request or for the whole session; the pause per session after it fires |

`when` filters on the event's fields before any judge: `tools`, `input`
(`contains`, `excludes`), `result`, `agent` (`top-level` or `subagent`),
`reason`, a permission's `actions`, `host` and `resources`, and `history`:
the tools called, the integration events seen and the session's status, as
far back as the latest request or the whole session. [Rule files: v1 to
v2](../migration/rule-format.md) maps each v1 field.

Names in `judges` and `asks` are referred to only by a step's `judge` and
`ask`. Step IDs are referred to only by `after`, `select` and `then`.

Rules load from four sources, and edits apply without a restart:

| Source      | Folder                                                                          | System Two                                                 |
| ----------- | ------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| Shipped     | `distribution/rules/`                                                           | Allowed                                                    |
| Integration | `~/.config/chauffeur/rules/<integration>/`, installed by the integration        | Allowed                                                    |
| User        | `~/.config/chauffeur/rules/`                                                    | Allowed                                                    |
| Project     | `.chauffeur/rules/`, from the Git worktree root down to the session's directory | Only where `~/.config/chauffeur` allows it for the project |

A rule's ID is its path under its folder, such as `sourcefed/triage`. The user
tunes an integration's rule by editing its file; reinstalling the integration
keeps a file the user changed.

### Commit messages

```jsonc
// distribution/rules/conventional-commits.json
{
  "on": "tool.after",
  "when": { "tools": ["shell"], "input": { "contains": "git commit" } },
  "judges": { "quick": { "tier": "system-one", "confidence": 0.7 } },
  "strategy": [
    {
      "id": "nonconventional",
      "judge": "quick",
      "ask": {
        "outcomes": ["yes", "no"],
        "evidence": ["tool_call"],
        "instructions": "Does this call create a git commit whose message does not follow Conventional Commits?"
      }
    }
  ],
  "then": [
    {
      "if": { "nonconventional": "yes" },
      "message.append": {
        "parts": [
          {
            "text": "Use Conventional Commits, such as `feat(scope): short description`."
          }
        ],
        "delivery": "now"
      }
    }
  ],
  "cooldown_seconds": 120
}
```

A rule on a harness event names [actions](../plugins/writing.md#actions) in its `then`, and the
daemon passes them to the adapter. A rule on a caller's event, such as
`sourcefed.event` or `opencode-history.recall`, returns intents the caller defines, such as
`withhold` and `attach`.

### Event triage

sourcefed installs this rule with its System One screen. This user added the
System Two review for Slack messages that System One leaves unsure or fails
on.

```jsonc
// ~/.config/chauffeur/rules/sourcefed/triage.json
{
  "on": "sourcefed.event",
  "judges": {
    "quick": { "tier": "system-one", "confidence": 0.7 },
    "reasoner": { "tier": "system-two", "confidence": 0.6 }
  },
  "asks": {
    "needs-agent": {
      "outcomes": ["yes", "no"],
      "evidence": ["thread", "event"],
      "instructions": "Does the new event ask the agent to do or change something?"
    }
  },
  "strategy": [
    { "id": "screen", "judge": "quick", "ask": "needs-agent" },
    {
      "id": "is-slack",
      "judge": "facts",
      "ask": { "match": { "field": "event.source", "equals": "slack" } }
    },
    {
      "id": "review",
      "judge": "reasoner",
      "ask": {
        "outcomes": ["act", "ignore"],
        "evidence": ["thread", "event"],
        "instructions": "Should the agent act on the new message? Cite the evidence."
      },
      "after": [
        { "step": "screen", "when": ["unsure", "failed"] },
        { "step": "is-slack", "when": ["yes"] }
      ]
    }
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
  "on": "opencode-history.recall",
  "strategy": [
    {
      "id": "helps",
      "judge": "system-one",
      "for_each": "candidates",
      "ask": {
        "outcomes": ["yes", "no"],
        "evidence": ["message", "candidate"],
        "instructions": "Would this passage materially help the agent answer the message, beyond what it already says?"
      }
    },
    {
      "id": "best",
      "judge": "facts",
      "after": ["helps[*]"],
      "ask": { "select": { "from": "helps[*]", "top": 3, "min": 0.7, "by": "yes" } }
    }
  ],
  "then": [{ "attach": "best" }]
}
```

`select` ranks the copies by their probability of `yes`. All candidates share
one round, so Jev answers them in one call. An error or a missed deadline
attaches nothing.

### What a caller gets back

| Result         | Meaning                                                                                                                              | `navigate`                                                                  |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------- |
| Intents        | Every matching rule ran                                                                                                              | `intents`: each with its `rule`, possibly empty                             |
| Verdict        | An inline strategy's steps finished                                                                                                  | `verdict[id]`: `{ "answer": … }`, `{ "failed": "<reason>" }` or `"skipped"` |
| Unknown judge  | An inline strategy names a tier with no model, or one its source may not use; nothing ran. A rule that does is refused when it loads | `judge_unavailable` error                                                   |
| Limit exceeded | The strategy or evidence is over a named limit; nothing ran                                                                          | `limit_exceeded` error                                                      |
| Busy           | This source's or the daemon's capacity is in use                                                                                     | `busy` error                                                                |
| Deadline       | The deadline passed; unfinished steps are unanswered                                                                                 | `deadline` error, with the events so far                                    |
| Unavailable    | The daemon is not running                                                                                                            | `unavailable` error                                                         |

## Plugins

A plugin is a long-running TypeScript program that hooks into events, asks
questions and acts on the answers: through the harness, such as steering the
agent, or by itself, such as writing a file or starting a background task.
The daemon starts it, passes it the events it hooks, and carries its harness
actions to the adapter. Plugins are harness-agnostic:
the same plugin works in every harness whose adapter supports the actions it
uses.

The [plugins guide](../plugins/README.md) has a page for each first-party
plugin, and [Writing a plugin](../plugins/writing.md) covers the SDK, hooks,
actions, questions, options, storage and lifecycle. The decisions they rest
on:

| Concern      | Decision                                                                                                                                                             |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Hooks        | `ctx.on` never holds the harness; `ctx.before` holds it until its handler returns or its budget passes, then applies its `onTimeout`                                 |
| Questions    | `ask` and `strategy` send inline strategies through `navigate`, so confidence, fallback, redaction and keys stay in the engine; a plugin never holds a model key     |
| Actions      | Named by resource and verb, carried out by the harness package, retried by kind, and checked against the session's `revision` so a stale one never lands             |
| Process      | A child of the daemon, speaking JSON-RPC 2.0 over stdio; lazy, stopped after 10 idle minutes unless eager; a crash restarts it with backoff and affects nothing else |
| Discovery    | `setup` runs once to record the hooks, cached against the plugin's file hash, and again when its files change                                                        |
| Registration | `~/.config/chauffeur/plugins.json`, keyed by ID; a project can never add a plugin                                                                                    |
| Commands     | `ctx.command` adds `chauffeur <plugin> <command>`, recorded at discovery and run in the plugin; a plugin cannot take a built-in command's name                       |
| Tools        | `tool.add` gives the agent a tool through the harness's registry. [Tools](../tools/README.md) shows how agents call them                                             |
| State        | `ctx.storage`, the plugin's own folder under `~/.local/state/chauffeur/plugins/`                                                                                     |

### Events

Harness events and actions are the [harness package's](#adapters) contract:
its schemas define them, and its fidelity table says which each harness
supports. Chauffeur reads those schemas; plugins and rules hook events by
name, such as `tool.after`, `tool.before` and `prompt.sent`.

A caller's or plugin's own events are keyed by its name, before a dot:
`sourcefed.event`, `opencode-history.recall`, `neo.screen-check`. A rule's
`on` therefore says whose event it hooks, and two programs never share an
event by accident. The harness package's namespaces (`tool`, `prompt`,
`permission`, `response`, `session`, `compaction`) are reserved. The daemon
refuses a plugin's event outside the plugin's own namespace: only the
`sourcefed` plugin sends `sourcefed.*`, and no other plugin can pose as
sourcefed.
Programs on the socket are the user's own, as in
[Access, delivery and failure rules](#access-delivery-and-failure-rules).

## Adapters

Adapters come from the shared harness package, a separate project that any
program can use to hook a harness and act on it. It holds the contract as
data (event and action schemas, the fidelity table, the rules for combining
several deciders' answers), one adapter per harness, and a hook command for
harnesses that only run commands. It holds no policy.

Chauffeur is one decider among others, such as sourcefed and Workhorse:

```mermaid
flowchart LR
    H["Harness"] --> A["Harness package adapter"]
    A -- "events, before" --> C["Chauffeur daemon<br/>rules and plugins"]
    A -- "events, before" --> O["Other deciders"]
    C -- "actions" --> A
```

| Chauffeur needs           | From the package                                                                                                        |
| ------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| Events and `before` hooks | Delivered to the daemon's socket; the daemon routes them to its rules and plugins and answers within each hook's budget |
| Actions                   | The daemon sends its rules' and plugins' actions through the package, which carries them out or returns `unsupported`   |
| The contract              | The package's schemas and fidelity table, which `crates/protocol` reads                                                 |

Chauffeur keeps no per-harness code. A harness slice adopts the package's
adapter for that harness and proves every feature so far through it.

## Rulebooks

Status: a sketch, held for discussion before its slice starts.

A rulebook is a set of rules that stays off until the user starts it in a
session, such as `/goal ship the parser fix`. A book is data in the rule-file
shape, and the first-party `rulebooks` plugin runs it:

```jsonc
// distribution/rulebooks/goal.json
{
  "id": "goal",
  "args": { "required": true },
  "budget": 20,
  "on_start": "Goal: {args}. Work toward it until evidence shows it is done.",
  "rules": [
    {
      "on": "response.finished",
      "judges": { "quick": { "tier": "system-one", "confidence": 0.7 } },
      "strategy": [
        {
          "id": "done",
          "judge": "quick",
          "ask": {
            "outcomes": ["yes", "no"],
            "evidence": ["summary", "todos"],
            "instructions": "Is {args} done and checked?"
          }
        }
      ],
      "then": [
        { "if": { "done": "yes" }, "end": "complete" },
        {
          "if": { "done": "no" },
          "message.append": {
            "parts": [{ "text": "Not done yet: {args}" }],
            "delivery": "wake"
          }
        }
      ]
    }
  ]
}
```

- The plugin registers `/goal` with `command.add`, and keeps each session's
  running books, their arguments and remaining budget.
- When a running book's event fires, the plugin runs its rules through
  `navigate` and carries out their `then`, counting each delivery against the
  budget.
- Books load from `distribution/rulebooks/`, `~/.config/chauffeur/rulebooks/`
  and `.chauffeur/rulebooks/`, like rules.

## Proofs

| What                             | How                                                                                                                                                                                                               |
| -------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `after` and rounds in core       | Kani: a step waits for its needs, decisions follow the rule, a run never stalls and finishes within one round per step                                                                                            |
| The runtime                      | Miri: deadlines, cancellation, late results, overflow and capacity                                                                                                                                                |
| Shipped facts                    | Exhaustive tests over small inputs, run in the QuickJS sandbox: for example, `select` with `top` and `min` keeps at most `top` candidates, each at or above `min`, best first, for every input up to 4 candidates |
| Confidence                       | Kani: for every probability in thousandths and every valid confidence, an answer settles to exactly one outcome, edges inclusive                                                                                  |
| TypeScript checks before actions | gdp-ts: an adapter carries out an action only with a `RevisionIsCurrent<Session, Revision>` proof, and an action the host gates only with a `HostAllowed<Action>` proof. Its lint preset rejects forged proofs    |

## Access, delivery and failure rules

- The daemon listens only on a Unix socket in a directory only the user can
  open (`chauffeur/` in the user's runtime directory, mode `0700`), so only the
  user's own processes connect. There is no TCP port and no token.
- The daemon runs as a Homebrew service when the user starts one. A second
  start that finds the socket taken exits.
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

A judge answers a step's request. Core defines the interface: synchronous
`judge`, and `judge_all` for a batch, which a batching provider overrides. Any
closure is a judge:

```rust
let panel = Panel::new().with("facts", |ask: &Ask| Value(Yes)).with("system-one:jev", &jev);
let verdict = plan.run(&panel)?;
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

Each slice adds the tab for its feature:

| Tab      | Slice | Contents                                                                                                                                                                                                                            |
| -------- | ----- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Question | 1     | A question's outcomes, probabilities and confidence; the settled outcome, `unsure` or `failed`                                                                                                                                      |
| Strategy | 3     | A strategy editor with an example for each row of the [strategies table](#strategies); validation errors; each round's steps, with their outcome or failure picked by hand; the event timeline and verdict; the strategy in the URL |
| Facts    | 4     | A fact's input and its answer, with its memory and time used                                                                                                                                                                        |

The page calls the same Rust code as the daemon, compiled to WASM. The Vite
development loop rebuilds Rust changes in `playground/wasm/` and `crates/`,
and reloads the page.

## Package layout

The product holds building blocks only. Shipped content lives in
`distribution/` and loads from a path, through the same loader as the user's
own; nothing in `crates/` embeds it.

```mermaid
flowchart LR
    subgraph Product["Product: building blocks, no content"]
        Daemon["chauffeur daemon run<br/>loads sources from paths"]
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
  cli/           # chauffeur binary: daemon, emit, audit, navigate, strategy,
                 #   and each plugin's own commands
  o11y/          # Event consumers, on tracing
packages/
  chauffeur/     # @fdcn/chauffeur: /client, /judge, /plugin, /harness
plugins/
  skills/            # Skill exposure and safety review, over the harness's skill catalog
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
site/            # The documentation site: every Markdown file, on Cloudflare
scripts/         # Packaging, Homebrew formula, smoke tests
.github/workflows/
```

| Rule                           | Consequence                                                                                                                                                         |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/` never embeds content | A build without `distribution/` still runs, with nothing in it                                                                                                      |
| One loader for every source    | The shipped `select.js` takes exactly the path a user's `notes.js` takes                                                                                            |
| Precedence                     | Shipped, then user; project content is limited (pure facts, no models, System Two only if allowed)                                                                  |
| Install                        | Packaging copies `distribution/` to `~/.local/share/chauffeur/`, so the product and its content release separately                                                  |
| Plugins                        | `plugins/` holds the first-party plugins. Each is its own package on the SDK, registered in `plugins.json` like anyone else's; nothing in `crates/` depends on them |

Each crate arrives with the first slice that needs it. `o11y` waits until the
core event schema is stable.

## Sequence and verification

Each slice delivers one feature a person uses, through the real path, and
ends with review. [Slices](slices/README.md) has a page for each: what you can
do with it, the files it adds, and how it is verified.

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

| Concern                      | Decision                                                                                                                                                                                                                                                                                                                                                                                                                           |
| ---------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Wire protocol                | JSON-RPC 2.0 over the Unix socket and stdio                                                                                                                                                                                                                                                                                                                                                                                        |
| First-party plugins          | `plugins/` in this repository, each its own package on the SDK. An integration's plugin lives in the integration's repository and is installed with its rules, such as sourcefed's                                                                                                                                                                                                                                                 |
| Adapters                     | Adopted from the shared harness package, designed separately; Chauffeur registers as a decider and reads the package's event, action and fidelity data. Slice 1's OpenCode part and its rc wait for the package                                                                                                                                                                                                                    |
| Chauffeur for other services | Any program asks through `navigate`, with no harness: sourcefed, Workhorse, neo, opencode-history and scripts. Each passes through when Chauffeur is unavailable                                                                                                                                                                                                                                                                   |
| Slices                       | Each slice is one feature a person uses, built end to end and released as an rc; the pieces arrive with the first feature that needs them. Cutover waits for rulebooks                                                                                                                                                                                                                                                             |
| rc beside v1                 | The `chauffeur-next` Homebrew formula installs `chauffeur-next`, with its own socket and `~/.config/chauffeur-next/`, so v1 keeps running untouched until cutover                                                                                                                                                                                                                                                                  |
| `bm25` and `embedding` facts | Ship with the first feature that uses them                                                                                                                                                                                                                                                                                                                                                                                         |
| Jev's `score` type           | `choice` for every question that is not yes/no; slice 3 measures `score` on labelled scale cases, and a question gains a hint only if `score` is clearly better                                                                                                                                                                                                                                                                    |
| Production dependencies      | Core stays dependency-free. Approved: `serde` and `serde_json` (protocol and every config file), `thiserror` (typed errors outside core), `rquickjs` and bundled `rusqlite` (facts and the `chauffeur:sqlite` host module), `notify` (reloading rules, facts and models), `ureq` with rustls (model formats, from slice 1), `tracing` and `jiff` (o11y events and audit timestamps), `clap` and `color-eyre` (the CLI binary only) |
| Development tools            | `cargo-nextest` runs tests (doctests stay on `cargo test --doc`); `cargo-mutants` checks that tests catch changed logic in core, confidence and lowering; `cargo-shear` catches unused dependencies; `criterion` benchmarks the socket hop, confidence, a fact call and lowering; `bacon.toml` holds shared watch jobs                                                                                                             |
| blaze                        | Builds and tests in the local loop, through `bacon`. Clippy, CI, Kani, Miri and `cargo-mutants` run on cargo with the pinned toolchain, which stays the final word: blaze builds with its own nightly and its borrow checker is incomplete                                                                                                                                                                                         |
| scriptc                      | A slice 5 spike for first-party plugins; the engine stays Rust                                                                                                                                                                                                                                                                                                                                                                     |

## Open questions

| Question                                                                                                                                                                                        | Recommended default                                                                                            |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| The rulebooks design                                                                                                                                                                            | Discuss before slice 12; the [sketch](#rulebooks) is the starting point                                        |
| How the daemon starts when no service runs and only the harness package connects                                                                                                                | Settle with the harness package's decider registration                                                         |
| The harness package carries `request`, `activity` and `history` with each event, `session.update` with a session's hidden tools, and `session.warm` in OpenCode as a transient generate request | Ask for all three in the package's contract before slice 5; v1's rules, the tools plugin and warming need them |
| Whether jev-distill students may be trained on Jev's labels, given TypeSafe's agreement §2.3(b)                                                                                                 | Settle in jev-distill before Chauffeur ships a student in a rule                                               |
