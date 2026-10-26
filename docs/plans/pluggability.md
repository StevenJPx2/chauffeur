# Chauffeur v2 implementation plan

Status: design for review. The starting workspace is clean apart from its
toolchain, license, plan/audit documents and the Vite/WebAssembly playground.
The [v1 architecture audit](../audits/architecture.md) records the source
findings and checks that inform this plan.

## Goal

Chauffeur's core runs generic, bounded judgment plans. Each step names the
tier of judge that answers it: facts (deterministic code), System One, System
Two or a person. Every judge takes the same question and returns the same
answer. Model providers are configuration; only each API format is code.

Callers such as sourcefed and opencode-history send a question with its
context through `chauffeur.navigate`. Rule files, which are data, decide how to
answer: when to ask, the plan to run and what to do with the outcome. Chauffeur
never runs code from a repository, and no caller needs a token.

The initial slices are:

1. a generic state machine that is playable in the browser;
2. the plan protocol, the judge contract, rule files, the runtime and the
   daemon, reached through `chauffeur.navigate`;
3. System One providers: Jev, SubQ and GLiDE;
4. the first System Two provider on the same plan/runtime path;
5. behavior plugins and the OpenCode adapter, reaching parity with `main`;
6. release and cutover: v2 replaces v1 as the shipped Chauffeur.

The work proceeds one crate/slice at a time, with review before advancing.

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
| Version mixing | The daemon/plugin version handshake keeps v1 and v2 from running together |
| User state | `~/.config/chauffeur` is migrated, or read compatibly, at cutover |
| After cutover | `v1` is a branch for hotfixes only |

### Parity table

Branch point `73bc29e`. Watermark: `e7fbabf`.

| `main` commit | v2 home | Slice | Evidence from `main` |
|---|---|---|---|
| `f2b087b` judge interface, facts judge | Facts is a rule judge on the panel; `judge.json` routes become step judge names | 2 | `judges/facts/src/lib.rs` tests |
| `bb41c9a` context renewal, session warming | Model-router behavior; the adapter performs `compact` and `warm` | 5 | `routing.rs`, `renewal.test.ts`, `warming.test.ts` |
| `22e5391`, `36f6253` skill safety review | Skill-review behavior: a screen-then-review plan | 3–4 | `sources/tests.rs` vetting cases |
| `3a18675` Skilld discovery | Skill-sources behavior; fetches run on runtime workers (audit A4) | 2, 5 | `cli/tests/skills.rs` |
| `c43ecd6` read guard, RPC starvation fix | Permission behavior; runtime admission permits (audit A3) | 2, 5 | `read-guard-probes.json`, `rpc-starvation.test.ts` |
| `9b8e917` rules | Rule content for the rules behavior | 5 | `capabilities/rules/tests/rules.rs` |
| `73bc29e` Homebrew release, CI | Release scripts and `check.yml` in v2 | 6 | `release.test.mjs`, `smoke.mjs` |
| `f54db24`, `d66a385`, `e7fbabf` docs | Documentation only | — | — |
| In progress on `main`: GLiDE provider, confidence escalation | `glide.json` over the `systemone` format; escalation is a rule's strategy, with `system-one/jev` then `system-one/glide` then `system-two` | 3 | `judges/glide/tests/wire.rs`, `docs/judges.md` escalation cases |

Context renewal on v2:

```mermaid
sequenceDiagram
    participant A as OpenCode adapter
    participant B as Model-router behavior
    participant R as Runtime
    participant C as Core
    participant F as Facts judge
    A->>B: context_resumed (idle, tokens, window)
    B->>R: Plan with step renew_context, judge "facts"
    R->>C: Start
    C-->>R: Round 1 asks renew_context
    R->>F: Judge the facts
    F-->>R: Yes
    R->>C: Answer
    C-->>R: Done, with verdict and events
    R-->>B: Verdict
    B-->>A: Intent to compact with the summary model
    A->>A: Check revision, then session.compact
```

Warming asks no question: the behavior returns a `warm` intent without a plan.

## Dependency direction

```mermaid
flowchart TB
    Core["crates/core<br/>Generic plan state machine"]
    O11y["crates/o11y<br/>Observes core events"] --> Core
    Protocol["crates/protocol<br/>Plan, evidence and verdict JSON"] --> Core
    Runtime["crates/runtime<br/>Drives rounds and dispatches providers"] --> Core
    Runtime --> O11y
    Daemon["crates/daemon<br/>Socket, rule files, default plugins"] --> Runtime
    Daemon --> Protocol
    Judges["crates/judges<br/>Ask and Answer, cuts, tiers, providers"] --> Core
    Runtime --> Judges
    Playground["playground<br/>WASM + interactive feature tabs"] --> Protocol
```

`crates/core` has no crate dependencies: no async runtime, HTTP, serde, UI,
logging or plugin system. Error types use the standard library. Other crates
consume core's event data.

Adding crates does not by itself increase the shipped binary: linked code and
dependencies determine size. Each package owns a real boundary.

## Pluggability boundaries

Chauffeur and sourcefed both work in any harness, so no policy lives in a
harness adapter or a caller. A caller sends a question with its context;
Chauffeur's rules decide how to answer it. Adapters only translate.

| Kind | What it is | Written by | Examples |
|---|---|---|---|
| Rule file | Data: `on`, `when`, `plan`, `then` | Chauffeur, an integration, the user or a project | `conventional-commits`, `sourcefed/triage`, `opencode-history/recall` |
| Native behavior | Rust inside Chauffeur, for policy that needs Chauffeur's state or arithmetic | Chauffeur | Model router, skill exposure, permission contracts, following the agent's PRs |
| Caller | Any program that sends a question and its context | Anyone | sourcefed's daemon, opencode-history, host adapters, hook scripts |
| Host adapter | Per-harness glue: reports what happens and carries out intents | Chauffeur, one per harness | `adapters/opencode`, `adapters/pi` |

### `chauffeur.navigate`

Callers use one operation, `chauffeur.navigate`: the decision a chauffeur
makes before steering. A request names an event and carries its context;
every rule whose `on` and `when` match answers it, and the caller receives
their intents:

```ts
import { connect } from "@fdcn/chauffeur-client"

const { intents } = await connect().navigate(
  { on: "recall", evidence: { message }, candidates: { p1: "…", p4: "…", p7: "…" } },
  { signal: AbortSignal.timeout(1_500) },
)
// intents → [{ rule: "opencode-history/recall", attach: ["p4", "p1"] }]
```

```mermaid
flowchart LR
    Callers["Callers<br/>sourcefed, opencode-history,<br/>host adapters, scripts"] -- "event and context" --> Daemon["Chauffeur daemon"]
    Rules[("Rule files<br/>built-in, integrations,<br/>user, project")] --> Daemon
    Daemon --> Strategy["Each matching rule's plan<br/>facts, System One, System Two"]
    Strategy -- "intents" --> Callers
```

- **The strategy is the rule's.** A rule can screen with facts, classify with
  System One and review only what System One leaves unsure with System Two.
  Changing the strategy changes a rule file, never a caller.
- **Answers are yes/no, a pick or a ranking**, built from the pieces in
  [Strategies](#strategies).
- **No intent means the caller's default.** A rule that matches nothing in its
  `then`, or whose judges fail, returns no intent: sourcefed then delivers the
  event, and recall attaches nothing.
- **An inline `plan` replaces `on`** for scripts, the playground and tests. It
  runs only that plan and returns its verdict.

Native behaviors answer the same events as rule files. The model router, for
instance, answers `context_resumed` with a `compact` intent.

#### sourcefed: one triage for every host

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
delivers, as v1's gate does. In the other direction, the native behavior that
follows the agent's PRs calls sourcefed's daemon API from a runtime worker,
never from the coordinator (audit A4).

#### opencode-history: recall

opencode-history searches its own index when the user sends a message, then
navigates `recall` with the message and the passages as candidates. Its rule,
`opencode-history/recall`, ranks them, and opencode-history attaches the
passages the `attach` intent names.

| Owner | Responsibility | Boundary |
|---|---|---|
| `crates/core` | Generic plan composition, validation and state transitions | No transport, provider, host or domain policy |
| `crates/judges` | The `Ask` and `Answer` contract, cuts, tiers, facts providers, API formats and provider configuration | No behavior policy |
| `crates/runtime` | Admission, run state, deadlines, cancellation, egress redaction and provider dispatch | Injects implementations; owns no behavior policy |
| `crates/protocol` | One JSON form of requests, plans, evidence, events, verdicts and intents for rule files, `navigate` and the playground | Validates and limits before anything runs |
| `crates/daemon` | The socket, rule-file loading and matching, and the default behaviors | Listens only on a socket the user owns |
| Behavior plugins | Questions, evidence selection, domain state, verdict interpretation and action intents | Never call host APIs directly |
| Host adapters | Report what happens with `navigate`, carry out intents, and expose `navigate` to their host's plugins | No policy; host denials remain authoritative |

### One protocol, many bindings

`navigate` is a Chauffeur operation, not any host's. Its request and result
are defined once, in `crates/protocol/schema/navigate.json`, and every binding
forwards them to the daemon socket unchanged:

| Binding | Caller | Mechanism |
|---|---|---|
| `@fdcn/chauffeur-client` | Any TypeScript program: sourcefed's daemon, host plugins, scripts | `connect().navigate(request)` over the socket |
| `chauffeur navigate` | Hook scripts in any host, Git hooks, CI on the same machine | One request on stdin; JSON lines on stdout |
| `crates/protocol` | Rust programs | The same types the daemon uses |
| `adapters/opencode`, `adapters/<host>` | That host's plugins, as a convenience | The host's plugin-to-plugin mechanism, such as OpenCode plugin RPC: `ctx.rpc(Chauffeur).navigate(request)` |

Adding a host or a language adds a binding, never a new request format. The
schema's fixtures run in the protocol crate's tests and in each binding's
tests.

### Stateless invocation

A navigation has a caller, request ID, an event or inline plan, bounded
evidence and candidates, and a deadline. It runs without an agent session,
`Signal`, `Situation`, or prior history; the caller supplies the evidence it
wants considered. Intents and transition events return to that caller. The
only state kept between navigations is a rule's cooldown and a native
behavior's own state. Only a host adapter carries out host actions.

### Judges

Every judge takes one `Ask` and returns one `Answer`. Judges come in four
tiers, each with one or more providers. A step names a tier, and may name the
provider too:

| `judge` | Answered by |
|---|---|
| `"system-one"` | The tier's providers, in the user's fallback order |
| `"system-one/glide"` | GLiDE; the tier's fallback order applies if GLiDE is unavailable or not configured, and the run's events say so |

Naming the provider lets one plan use two models of the same tier, such as
Jev for a fast screen and GLiDE, more accurate but slower, for what Jev leaves
unsure. A rule shared with someone who lacks GLiDE still runs, on their
System One.

```mermaid
flowchart TB
    Step["Plan step<br/>judge: system-one/glide, cut"] -- "Ask and evidence" --> Tiers
    subgraph Tiers["Tiers: one Ask in, one Answer out"]
        Facts["facts<br/>match, select, BM25, embedding"]
        S1["system-one<br/>Jev, SubQ, GLiDE, a local model"]
        S2["system-two<br/>an OpenAI or Anthropic model"]
        Person["person<br/>the user, through the host"]
    end
    Tiers -- "Answer" --> Cut["Runtime applies the step's cut"]
    Cut -- "outcome or failed" --> Plan["Plan: gates and then"]
```

#### The contract

| Type | Shape |
|---|---|
| `Ask` | `question`: `"yes-no"` or `{"choose": [labels]}`; `instructions`; `evidence`: names of evidence blocks |
| `Answer` | `score`, 0 to 1000; `label`, for a choice; `cites`, the evidence names it relied on. Or failed |

For a yes/no question the score is how strongly the answer is yes. For a
choice, the label is the judge's pick and the score its confidence. Every tier
answers both kinds, so moving a step between tiers changes only its `judge`.
`crates/judges` implements core's `Judge` interface for every tier.

#### The cut

The step, not the judge, decides what an answer means. The runtime applies
the step's cut the same way for every tier, and judges never see it:

| Question | Cut | Outcome |
|---|---|---|
| Yes/no | none | The score |
| Yes/no | `"cut": {"yes": 700, "no": 300}` | `yes` at 700 or above, `no` at 300 or below, `unsure` between; `no` must be below `yes` |
| Choice | none | The label |
| Choice | `"cut": {"min": 600}` | The label at 600 or above, `unsure` below |

A judge that cannot answer settles as `{"failed": true}` whatever the cut, so
a failure never reads as `no`. Each cut is tuned in its rule file against
labelled cases.

#### Tiers

| Tier | Answers with | Built-in providers |
|---|---|---|
| `facts` | Deterministic code: no model, network or credentials | `match` (equals, contains or a pattern over evidence), `select` (combines earlier outcomes; see [Strategies](#strategies)), `bm25` (rank over the step's evidence), `embedding` (similarity from the shared local embedding service) |
| `system-one` | A fast classifier, one batched call per round | `jev` (default), `subq`, `glide` |
| `system-two` | A reasoning model, with a schema-checked answer that cites its evidence | Disabled until reviewed (slice 4) |
| `person` | The user, asked through the host adapter | Each host adapter |

#### Providers are configuration

A provider is registered the way a user registers a model provider anywhere
else: a built-in API format plus the endpoint, model and credential to use.
Code exists only once per API format, in `crates/judges`:

| Format | Speaks |
|---|---|
| `systemone` | The `/v1/systemone` decisions API that Jev, SubQ Snap and Fastino GLiDE share; a provider entry sets GLiDE's reading of `score` as a level |
| `openai` | OpenAI-compatible chat completions with structured output, which covers local servers such as Ollama |
| `anthropic` | Anthropic Messages with schema-constrained output |
| `replay` | Answers recorded in a file, for tests and the playground |

```jsonc
// ~/.config/chauffeur/providers.json
{
  "providers": {
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

- Chauffeur ships `jev`, `subq` and `glide` as the same JSON in
  `crates/judges/providers/`. The user's file adds providers and overrides
  shipped ones by name. Selecting a provider is configuration and takes effect
  without a restart.
- `tiers` lists providers in fallback order. The next provider answers only
  when one is unavailable, rate-limited or out of time, never because of what
  it answered. Escalating to another tier is the plan's job, not fallback's.
- Providers load only from shipped and user configuration, never from a
  project's `.chauffeur/`. A provider is a network endpoint, so a cloned
  repository could otherwise send evidence wherever it chose.
- A key is named, as an environment variable or a keychain entry, and is read
  only by `crates/judges`. It never reaches rule files, callers or audit.
- A shipped provider is used once its key is present; a System Two provider
  also needs the user to turn it on.
- The `embedding` facts provider uses a shared, bounded local embedding
  service. Caches and ranking that belong to one behavior stay with it.

### Strategies

A strategy is a rule's `plan`, built from four pieces. Chauffeur has no list
of built-in strategies: any combination of the pieces is one.

| Piece | What it does |
|---|---|
| Step | Asks one judge one question; its cut turns the answer into an outcome |
| Gate | Runs a step only when earlier outcomes match |
| `for_each` | Copies a step once per candidate, in one round; inside a copy, `step[item]` names the same candidate's copy of an earlier step |
| Earlier outcomes | A step may read earlier steps' outcomes as its evidence |

Combining answers is a step too. The `facts` provider `select` reads earlier
outcomes and settles to one: `first` settled in a list, `top` N, all at or
above `min`, `majority`, or `mean`. A new way to combine is a new facts
provider. The protocol lowers `for_each` and `[item]` to ordinary core steps
and gates, so core runs every strategy alike.

| Strategy | Built from |
|---|---|
| Escalate | A step gated on an earlier step's `unsure` or failure |
| Rank | A `for_each` step, then `select` with `top` and `min` |
| Collect all that pass | A `for_each` step, then `select` with `min` |
| Check the survivors | Two `for_each` steps, the second gated on `first[item]` being `yes` |
| Vote | Parallel steps, `select` with `majority`, and a review gated on `unsure` |

Escalating from Jev to GLiDE, then to System Two:

```jsonc
"plan": { "steps": [
  { "id": "fast",   "judge": "system-one/jev",   "cut": { "yes": 700, "no": 300 }, "ask": { … } },
  { "id": "slow",   "judge": "system-one/glide", "cut": { "yes": 700, "no": 300 }, "ask": { … },
    "gates": [{ "mode": "any", "sources": ["fast"], "on": ["unsure", { "failed": true }] }] },
  { "id": "review", "judge": "system-two",       "cut": { "yes": 700, "no": 300 }, "ask": { … },
    "gates": [{ "mode": "any", "sources": ["slow"], "on": ["unsure", { "failed": true }] }] },
  { "id": "answer", "judge": "facts", "ask": { "select": { "first": ["review", "slow", "fast"] } } }
] }
```

A plan is a graph without cycles, so every strategy has a known maximum of
rounds and calls before it starts. Nothing loops or retries until confident.

The v1 experiment's direct `judge(policy)` and `complete(profile)` calls are
not the primary v2 plugin API: each exposes only one inference request and
cannot express a screen, conditional escalation, fan-out or result assembly.

#### Viewing a strategy

`chauffeur strategy show` prints a rule's plan as a tree, with each step's
round, its provider as the user's configuration resolves it, its gates and its
cut:

```text
$ chauffeur strategy show opencode-history/recall
opencode-history/recall                        on recall
├─ round 1  helps × candidates  system-one → jev
└─ round 2  best               facts select   top 3, min 700, from helps
then  attach best
at most 2 rounds · 1 provider call · 31 candidates at most
```

`--file` shows an unsaved rule, and an unknown or unconfigured provider is
marked where it appears. The playground draws the same tree from rule JSON and
steps through it with recorded answers.
A future one-step convenience API is acceptable only if it lowers to that same
path and cannot bypass plan limits, redaction or deadlines.

### Access, delivery and failure rules

- The daemon listens only on a Unix socket in a directory only the user can
  open (`chauffeur/` in the user's runtime directory, mode `0700`), so only the
  user's own processes connect. There is no TCP port and no token.
- `.chauffeur/` holds data only. Chauffeur never runs code from a repository.
  A cloned repository's rules are data the user did not write: they run under
  the same limits, and System Two stays off for them unless
  `~/.config/chauffeur` turns it on for that project.
- Callers are trusted code, as their host or the user runs them. They reach
  Chauffeur only through `navigate`; payloads are never authority.
- One connection carries one navigation: the request, then its events in
  order, then its intents or verdict. Each navigation's event buffer is bounded
  and overflow is an explicit event. Closing the connection cancels it.
- One admission permit covers queued, waiting and active work. Capacity per
  source (built-in, integration, user and project rules, and inline plans), plan size,
  evidence bytes, node count, depth, fan-out, rounds, attempts and output size
  are checked before dispatch.
- Deadlines include queueing, preparation, every inference round and delivery.
  A disconnected caller cancels future work; a blocking provider call retains
  its worker lease until it ends, and late results cannot commit actions.
- Redaction runs after evidence assembly and before every provider call, and
  also covers errors and audit metadata.
- A pipeline returns data and action intents, not host effects. A host adapter
  checks the current observation revision before acting; host
  denials and the safety veto cannot be overridden by a plugin verdict.

## Extension examples

Three rule files. A rule file needs no code: dropping it into a rules folder
is the whole install.

A rule file has four parts:

| Field | Meaning |
|---|---|
| `on`, `when` | The event that triggers the rule, and a cheap deterministic filter on it; a step with its own `when` is skipped when that filter fails |
| `plan` | The plan, in the same JSON as an inline plan and the playground |
| `then` | Intents by outcome, first match wins; no match returns no intent |

Rules load from four sources, and edits apply without a restart:

| Source | Folder | System Two |
|---|---|---|
| Built-in | Each behavior's directory in Chauffeur | Allowed |
| Integration | `~/.config/chauffeur/rules/<integration>/`, installed by sourcefed, opencode-history and others | Allowed |
| User | `~/.config/chauffeur/rules/` | Allowed |
| Project | `.chauffeur/rules/`, from the Git worktree root down to the session's directory | Only where `~/.config/chauffeur` allows it for the project |

A rule's ID is its path under its folder, such as `sourcefed/triage`. The user
tunes an integration's rule by editing its file; reinstalling the integration
keeps a file the user changed.

### Commit messages (rule file)

v1's `conventional-commits` rule, with its question as a one-step plan:

```jsonc
// .chauffeur/rules/conventional-commits.json
{
  "on": "tool_result",
  "when": { "tools": ["shell"] },
  "plan": { "steps": [{
    "id": "nonconventional",
    "judge": "system-one",
    "cut": { "yes": 700, "no": 300 },
    "ask": {
      "question": "yes-no",
      "evidence": ["tool_call"],
      "instructions": "Does this call create a git commit whose message does not follow Conventional Commits?"
    }
  }] },
  "then": [{ "if": { "nonconventional": "yes" }, "steer": "Use Conventional Commits, such as `feat(scope): short description`." }],
  "cooldown_seconds": 120
}
```

### Event triage (integration rule)

sourcefed installs this rule with its System One screen. This user added the
System Two review for Slack messages, which System One leaves unsure or fails
on.

```jsonc
// ~/.config/chauffeur/rules/sourcefed/triage.json
{
  "on": "integration_event",
  "plan": { "steps": [
    {
      "id": "screen",
      "judge": "system-one",
      "cut": { "yes": 700, "no": 300 },
      "ask": {
        "question": "yes-no",
        "evidence": ["thread", "event"],
        "instructions": "Does the new event ask the agent to do or change something?"
      }
    },
    {
      "id": "review",
      "judge": "system-two",
      "when": { "sources": ["slack"] },
      "gates": [{ "mode": "any", "sources": ["screen"], "on": ["unsure", { "failed": true }] }],
      "cut": { "min": 600 },
      "ask": {
        "question": { "choose": ["act", "ignore"] },
        "evidence": ["thread", "event"],
        "instructions": "Should the agent act on the new message? Cite the evidence."
      }
    }
  ] },
  "then": [
    { "if": { "screen": "no" }, "withhold": "event" },
    { "if": { "review": "ignore" }, "withhold": "event" }
  ]
}
```

Two rounds at most: `review` is skipped when the screen says `yes` or `no`, or
the event is not from Slack. Every other outcome, including a failed review,
returns no intent, and sourcefed delivers.

### Per-turn recall (integration rule)

opencode-history installs this rule. The rule ranks the passages its search
returned; the code gathers them and attaches the result.

```jsonc
// ~/.config/chauffeur/rules/opencode-history/recall.json
{
  "on": "recall",
  "plan": { "steps": [
    {
      "id": "helps",
      "judge": "system-one",
      "for_each": "candidates",
      "ask": {
        "question": "yes-no",
        "evidence": ["message", "candidate"],
        "instructions": "Would this passage materially help the agent answer the message, beyond what it already says?"
      }
    },
    { "id": "best", "judge": "facts", "ask": { "select": { "from": "helps", "top": 3, "min": 700 } } }
  ] },
  "then": [{ "attach": "best" }]
}
```

Its code is the [`navigate` call](#chauffeurnavigate) with a 1.5 s signal.
All candidates share one round, so Jev answers them in one call. An error or a
missed deadline attaches nothing.

### What a caller gets back

| Result | Meaning | `navigate` |
|---|---|---|
| Intents | Every matching rule ran | `intents`: each with its `rule`, possibly empty |
| Verdict | An inline plan's steps settled, failed or were skipped | `verdict[id]`: `{ settled }`, `{ failed: true }` or `{ skipped: true }` |
| Unknown judge | An inline plan names a tier with no provider, or one its source may not use; nothing ran. A rule that does is refused when it loads | `judge_unavailable` error |
| Limit exceeded | The plan or evidence is over a named limit; nothing ran | `limit_exceeded` error |
| Busy | This source's or the daemon's capacity is in use | `busy` error |
| Deadline | The deadline passed; unfinished steps are unanswered | `deadline` error, with the events so far |
| Unavailable | The daemon is not running | `unavailable` error |

## Core plan model

A plan is an ordered directed acyclic graph. It contains named steps, each
carrying a generic request payload `Q`; a caller chooses the outcome type `O`.
Core does not know what either means. A step may have several gates; every gate
must pass.

```text
Step<Q> = { id, judge: name, request: Q, gates: [Gate<O>] }
Gate<O> = { mode: any | all, sources: [preceding step IDs], on: [Outcome<O>] }
Outcome<O> = Value(O) | Failed
StepState<O> = Pending | Asked | Settled(Outcome<O>) | Skipped
```

`Skipped` is a machine state, not a classifier outcome. A failed provider call
settles asked steps as `Failed`; a user-defined outcome such as `Unsure` is a
normal `Value(O)`. A step runs at most once. Gates only reference preceding
steps, so a finite plan terminates after at most one round per step.

### Gate behavior

- **Any:** run as soon as one source settles to a value listed in `on`. Skip
  once all sources are terminal and none matched.
- **All:** skip as soon as a source is terminal without a listed value. Run
  once every source matches.
- A skipped source is terminal and is not a match. `Failed` can match when
  explicitly listed in `on`.

These are proposed semantics for review. Gates on one step combine with AND;
within a gate, `Any` and `All` short-circuit once their result is known.
Independent ready steps share a round.

### Judges

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

A step names its judge, because plans are data: rule files and inline plans
can only refer to judges the host already holds, and configuration can
limit which judges a project's rules may use. A `Panel` maps names to judges. `plan.run(&panel)` validates
the plan, rejects a name the panel lacks before asking anything, and then asks
each judge its share of every round in one `judge_all` call. A batch with the
wrong number of answers settles as `Failed`. The round-by-round API remains for
the playground and the runtime; each `Question` reports its step, judge and
request.

Judges are synchronous so rules stay plain functions. The runtime runs slow
judges such as Jev on bounded worker threads, with deadlines and cancellation;
neither core nor the interface changes for that.

### Composable plans

The public helpers are conveniences that lower to this same plan model:

```rust
let screen_then_review = ask("screen", "system-one", screen_question)
    .then_on(
        When::any([Outcome::Value(Unsure), Outcome::Failed]),
        ask("review", "system-two", review_question),
    );

let candidates_then_review = all([
    ask("skill-slack", "system-one", slack_question),
    ask("skill-jira", "system-one", jira_question),
    ask("skill-memory", "system-one", memory_question),
])
.then_if_any(Outcome::Value(Yes), ask("review", "system-two", review_question));
```

`ask`, `all`, `then_on` and `then_if_any` are ordinary functions/builders, not
a privileged list of built-in strategies. A plugin can define `fan_out`,
`screen_then_review`, or its own strategy by composing them. The playground
edits the same plan structure, so Rust builders, rule files and inline plans
have one meaning.

Composition lowers to the DAG: `all` combines independent plans;
`then_on`/`then_if_any` add a gate to each root of the following plan, referring
to the terminal steps of the preceding plan. If a step already has gates, the
composition gate joins that list. Names are unique across the composed plan.

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

Each transition includes:

- the next state: `Ask { round, questions }` or `Done { verdict }`;
- the events produced by the transition.

The proposed event vocabulary is `Started`, `RoundReady`, `StepSettled`,
`StepSkipped` and `Finished`. Events have stable step IDs, round numbers and
settled outcomes; they contain no transport or provider-specific types. A
runtime may offer a local observer callback around transitions; the only
user-supplied code core invokes is a judge, inside `run`.

`crates/o11y` consumes events for logs, traces or audit. `crates/runtime` can
offer a convenience observer callback, but core does not call user code. The
daemon serializes each run's events onto its caller's connection from a bounded
log; a slow caller never blocks a core transition.

The state machine does not decide provider choice, retry/backoff, HTTP,
credentials, or domain actions. Those belong to runtime, provider and plugin
contracts respectively.

## Playground

The Vite application uses petite-vue for reactivity and Web Awesome for UI
components. Each feature under test is a tab; its Rust implementation is
compiled to WASM.

The **State machine** tab will contain:

- A JSON plan editor with examples for single, chain, fan-out, screen/review,
  fallback and fan-out/review.
- A validation area for invalid IDs, gates or plan structure.
- The current ready steps, with controls to manually select a value or failure
  reply for each step.
- A transition-event timeline and final per-step verdict.
- Reset/replay controls and a sharable plan fragment in the URL.

The playground invokes the same core state transitions as the native runtime;
it does not contain a second implementation of gate semantics.

The Vite development loop rebuilds Rust changes in `playground/wasm/` and
`crates/`, and reloads the page. The State machine tab is the only tab.

## Package layout

```text
crates/
  core/                 # Generic plan, outcomes, judges, state machine and events
  o11y/                 # Event consumers
  protocol/             # Plan, evidence and verdict JSON; validation and limits
  judges/               # Ask and Answer, cuts, tiers, facts, API formats
    providers/          # Shipped provider configuration, such as jev.json
  runtime/              # Admission, deadlines, workers, redaction, transition driver
  daemon/               # Socket, rule loading and matching, default behaviors
  cli/                  # `chauffeur` binary: navigate, setup and audit
plugins/
  behaviors/             # Each owns its plans, config and shipped content
    model-router/        # Model switching, context renewal, warming
    skills/              # Skill sources, exposure and safety review
    permission/          # Permission contracts and guards
    rules/               # Built-in rule files
adapters/
  opencode/              # Reports what happens, carries out intents, `navigate` binding
  <host>/                # Each further host: the same three duties
clients/
  typescript/            # @fdcn/chauffeur-client: navigate over the socket
playground/
  wasm/                  # Thin wasm-bindgen bridge to core
  src/features/          # One interactive tab per feature under test
scripts/                 # Packaging, Homebrew formula, smoke tests
.github/workflows/       # check.yml
```

Shipped content lives with the behavior that reads it, so one directory owns
each rule, contract and config (audit A10).

The first core slice does not create provider, protocol, runtime or daemon crates.
They arrive when the core API they consume is reviewed. `o11y` also waits until
the core event schema is stable.

## Sequence and verification

### Slice 0 — playground shell: implemented

- Keep the pinned nightly, strict workspace lint setup, LICENSE and plan/audit
  documents.
- Use Vite, TypeScript and Bun; petite-vue provides reactivity and Web Awesome
  supplies tabs, cards and inputs.
- Compile Rust to browser WASM using wasm-bindgen/wasm-pack.
- Prove in a real browser that editing input updates Rust output, and that an
  edit to Rust rebuilds WASM and reloads the page.

### Slice 1 — core plan and state machine

Status: implemented and reviewed. Steps name a judge; core defines the
`Judge` interface, `Panel` and `Plan::run` (`crates/core/src/judge.rs`). The
playground seats a deterministic `keywords` rule judge beside `person`.

Files: `crates/core/Cargo.toml`, `crates/core/src/{lib,plan,gate,state,event}.rs`,
Kani harnesses in `crates/core/src/proofs.rs`, scenarios in
`crates/core/tests/scenarios.rs`. Playground:
`playground/wasm/src/state_machine.rs`,
`playground/src/features/state-machine.ts` and its template in
`playground/index.html`.

- Implement the generic plan graph, builder composition, `start`/`answer`
  transitions and transition events.
- Add small Kani harnesses separately for Any/All gate resolution, skip
  propagation, each step asked at most once and bounded termination. Do not
  combine plan shape, reply parsing and result interpretation into one proof.
- Add scenario tests for the playground strategies and invalid transition
  order. Keep each proof to one invariant, a small fixed graph and a small
  outcome enum.
- Run strict clippy, scenario tests, Kani, Miri, and the browser playground.
- Stop for user review before adding observability or the daemon.

Kani results, each run alone (`cargo kani -p chauffeur-core --harness <name>`):

| Harness | Property | Time |
|---|---|---|
| `any_gate_runs_on_a_match_and_skips_when_none_remains_possible` | `Any` gate decision | 83 s |
| `all_gate_runs_when_every_source_matches_and_skips_on_a_terminal_mismatch` | `All` gate decision | 150 s |
| `a_gate_decision_is_final` | Short-circuiting is sound | 44 s |
| `a_step_runs_only_when_every_gate_runs` | Gates on one step combine with AND | 188 s |
| `one_pass_settles_every_decidable_step` | Skips propagate within one pass | 488 s |
| `a_pass_only_moves_pending_steps_forward` | Each step is asked at most once | 240 s |
| `a_pass_with_no_open_round_asks_or_finishes` | A run never stalls | 156 s |

The last two give termination within one round per step; `proofs.rs` states the
argument. Gate proofs cover three sources and every outcome subset; whole-plan
proofs cover every three-step plan whose gates name one source and one outcome.

### Slice 2 — Navigate: protocol, judges, rules, runtime and daemon

A caller sends an event or an inline plan, with its evidence, candidates and
a deadline, and receives the events and the intents or verdict. It sends data,
never callbacks or code. This slice loads rule files from all four sources and
matches them by `on` and `when`. It builds `navigate` with three bindings so no
host or language shapes the request: the TypeScript client, `chauffeur
navigate`, and the OpenCode binding built on the client.

```mermaid
flowchart LR
    Program["TypeScript program"] --> Client["@fdcn/chauffeur-client"]
    Plugin["OpenCode plugin"] -- "plugin RPC" --> OpenCode["adapters/opencode"]
    OpenCode --> Client
    Script["Hook script"] -- "stdin" --> Cli["chauffeur navigate"]
    Client -- "Unix socket" --> Daemon["crates/daemon<br/>one navigation per connection"]
    Cli -- "Unix socket" --> Daemon
```

#### Wire contract

One JSON request in, events and then intents out as JSON lines. The socket
carries exactly what `chauffeur navigate` reads and prints:

```sh
chauffeur navigate <<'EOF'
{"protocol": 1, "deadline_ms": 1500, "on": "recall", "evidence": {"message": "…"}, "candidates": {"p1": "…", "p4": "…"}}
EOF
# → {"event": {"rule_matched": {"rule": "opencode-history/recall"}}}
# → {"event": {"round_ready": {"rule": "opencode-history/recall", "round": 1, "steps": ["helps[p1]", "helps[p4]"]}}}
# → {"event": {"step_settled": {"rule": "opencode-history/recall", "step": "helps[p4]", "outcome": 910}}}
# → {"event": {"step_settled": {"rule": "opencode-history/recall", "step": "best", "outcome": ["p4"]}}}
# → {"intents": [{"rule": "opencode-history/recall", "attach": ["p4"]}]}
```

A request with `plan` in place of `on` ends with `{"verdict": {…}}`.
`$CHAUFFEUR_SOCKET` defaults to `chauffeur/daemon.sock` in the user's runtime
directory: `$XDG_RUNTIME_DIR`, or `$TMPDIR` on macOS. A plugin-RPC binding
takes the same request as an object and returns the intents or verdict.

- **Outcomes** are a string, an integer or a list of candidate names, compared
  by equality. The runtime turns each `Answer` into an outcome with the step's
  [cut](#the-cut). Gates match outcomes, so a step that gates others has a cut.
- **Intents** carry the rule that produced them. A rule's intents are dropped
  if its plan misses the deadline; other rules' intents still return.
- **Verdicts** give each step `{"settled": <outcome>}`, `{"failed": true}` or
  `{"skipped": true}`. A gate names failure as `{"failed": true}`, so a judge's
  failure never collides with a label.
- **Evidence** is named once per navigation and referenced by steps, so a
  batched provider call carries each block once.
- **Versioning:** `protocol` is required; the daemon answers
  `unsupported_protocol` with the versions it accepts, and each binding reports
  it as `unavailable`.
- **Limits**, checked before admission: 32 steps per plan, counting each
  copy `for_each` makes; 16 gates per step; 64 evidence blocks and
  candidates; 256 KiB of evidence; 30 s deadline. Each rejection names its
  limit.

#### Files

| Path | Contents |
|---|---|
| `crates/protocol/src/{lib,request,plan,rule,intent,limits,error}.rs` | Serde wire types for requests, plans, rule files and intents; validation and limits; lowering plans, `for_each` copies and `[item]` gates to `chauffeur_core::Plan<Ask, Outcome>`; typed errors |
| `crates/protocol/schema/navigate.json`, `crates/protocol/fixtures/` | The one `navigate` schema every binding uses, and the valid and invalid requests each binding's tests replay |
| `crates/judges/src/{ask,cut,tiers,config}.rs` | `Ask` and `Answer`; applying cuts; resolving `tier` and `tier/provider` against shipped and user `providers.json` in fallback order, refusing any provider from a project |
| `crates/judges/src/{facts,replay}.rs` | The `match` and `select` facts providers, and the `replay` format, which answers from recorded scores for tests |
| `crates/runtime/src/admission.rs` | One permit per navigation, held from admission through delivery; per-source and global capacity |
| `crates/runtime/src/run.rs` | Drives rounds on a bounded worker pool; checks the deadline and cancellation before each round; discards late results |
| `crates/runtime/src/redact.rs` | Redacts evidence before every provider judge, porting v1's rules and their cases |
| `crates/runtime/src/events.rs` | Bounded per-run event log; overflow is an explicit event |
| `crates/daemon/src/{main,socket}.rs` | Creates the socket directory `0700` and refuses one it does not own; one navigation per connection, JSON lines out, closing cancels; `--providers FILE` replaces the user's providers, for tests |
| `crates/daemon/src/rules.rs` | Loads rule files from the four sources, refuses System Two in a project rule the user has not allowed, reloads on change, matches `on` and `when`, and runs matching rules concurrently |
| `crates/cli/src/navigate.rs` | `chauffeur navigate`: one request on stdin, the socket's JSON lines on stdout; exit 0 with intents or a verdict, 1 when rejected, 2 when the daemon is unavailable |
| `crates/cli/src/strategy.rs` | `chauffeur strategy show <rule>` and `--file`: the plan as a tree, with rounds, resolved providers, gates, cuts and the maximum rounds and provider calls |
| `clients/typescript/src/index.ts` | `@fdcn/chauffeur-client`: `connect().navigate(request, { signal })` over the socket, no dependencies; closing on abort cancels the navigation |
| `adapters/opencode/src/rpc.ts` | `Chauffeur`, the OpenCode binding defined from `navigate.json`, exported as `@fdcn/chauffeur/rpc`; forwards each call through the client |
| `playground/wasm`, `playground/src/features/strategy.ts` | Parses plans through `crates/protocol`, so the page, rule files and inline plans share one JSON format; draws a rule's strategy tree and steps through it with recorded answers |

#### Reference caller: opencode-history recall

[Per-turn recall](#per-turn-recall-integration-rule) is the reference caller:
its rule installed in `~/.config/chauffeur/rules/opencode-history/`, and its
code reduced to one `navigate` call. Slice 2 runs it end to end against
recorded scores.

#### Verification

- Protocol: round-trip and rejection cases for every limit and malformed field.
- Runtime under Miri: deadline, cancellation, late result, overflow, and
  exhausted capacity, including audit A3's flood: 81 submissions with capacity
  8 admit 8 and reject 73 with `capacity`.
- Kani: a permit is released exactly once on every terminal path; the event
  log never exceeds its bound; every score from 0 to 1000 maps to exactly one
  outcome under any valid cut, with both edges inclusive; `select` with `top`
  and `min` keeps at most `top` candidates, each at or above `min`, best
  first; every lowered plan has at most as many rounds as `strategy show`
  reports.
- Judges: fallback moves to the next provider only when one is unavailable,
  rate-limited or out of time; `system-one/glide` without GLiDE configured
  runs on the tier's fallback order and reports it; a provider in a project's
  `.chauffeur/` is refused; a key never appears in an event, error or verdict.
- Strategies: each row of the [strategies table](#strategies) runs as a rule
  with recorded answers; `[item]` gates skip exactly the candidates whose
  earlier copy did not match.
- Daemon socket: an unknown judge, an unsupported protocol, an oversized line
  and a closed connection each fail safely; events arrive in order; another
  user's socket directory is refused.
- Rules: each of the four sources loads; a project rule naming System Two is
  refused unless the user allows it; an edited rule applies to the next
  navigation; two matching rules both return their intents; a rule that misses
  the deadline drops only its own intents.
- All three bindings against the daemon with a `replay` System One: the
  playground examples return the same verdicts through the client, `chauffeur
  navigate` and a real OpenCode's plugin RPC as through `Plan::run`; every
  fixture is accepted or rejected identically by each; aborting any caller
  cancels the navigation.
- Recall: Jev scores recorded by opencode-history's replay feed a `replay`
  System One; navigating `recall` attaches the replay's top three passages
  scoring 700 or above. No agent session or history is sent.

### Slice 3 — System One providers: Jev, SubQ and GLiDE

Implement the `systemone` API format in `crates/judges` using the current
`/v1/systemone` contract, including authentication, timeout, bounded response,
explicit 429/529 handling and GLiDE's level-valued `score`. Ship `jev.json`
as the default System One, and `subq.json` and `glide.json` beside it. Core
and the runtime stay unaware of any provider.

Verification: loopback HTTP contract cases for each provider's documented
request/response, invalid replies and provider errors, replaying `main`'s
`judges/{jev,subq,glide}/tests/wire.rs`; then run the Jev-to-GLiDE escalation
from [Strategies](#strategies) against both with real keys, and compare its
answers with `main`'s `CHAUFFEUR_JUDGE_ESCALATE=glide` on the same cases.
Report the plan, answers, rounds, elapsed time and provider-call count. Run opencode-history's recall
replay live through `navigate`: one Jev call per message, judge time within
the replay's 278 ms p50 and 318 ms p95 plus local overhead.

### Slice 4 — System Two providers

Implement the `openai` and `anthropic` API formats in `crates/judges`. A System
Two answer is schema-checked, and its `cites` must name evidence the step was
given. Redaction applies before every call. Provider entries set model,
effort, timeout and output budget. System Two ships disabled until a provider
and its evaluations are reviewed.

Verification: loopback cases for valid, refused, malformed, truncated,
unavailable and timed-out replies; screen→review round ordering; citations of
evidence the step lacks are rejected; the 32 KiB input budget; and proof that a
slow reasoning call cannot consume reserved interactive System One capacity.
Then replay human-labelled cases before enabling a live provider.

### Slice 5 — Behaviors and OpenCode adapter parity

Port each parity-table row assigned to slice 5 as a native behavior or a
built-in rule file that answers navigations with intents. Each behavior moves
its rules, contracts and config into its own directory, and v1's rules convert
to rule files. Host adapters navigate what happens in the host (`tool_result`,
`turn_end`, `permission_request`, `context_resumed`) and carry out the intents.

v1's event gate becomes `sourcefed/triage`. In sourcefed, its daemon navigates
`integration_event` before queuing each event, its installer adds the rule,
and its OpenCode adapter's `chauffeur.gate` call goes. Before the slice closes,
review every `main` commit past the watermark and add its row.

Verification: each row's evidence from `main` passes against v2; every v1 rule
converts and validates; an intent for a stale observation revision is refused
at the adapter; sourcefed events reaching OpenCode, Pi and the CLI are
filtered alike, and v1's 12 labelled events give the same deliveries; the
bench `verify` fixtures pass; a day of real use through a locally built plugin
and daemon.

### Slice 6 — Release and cutover

Restore `check.yml`, `scripts/package.mjs`, `scripts/homebrew.mjs` and
`scripts/smoke.mjs` for the v2 layout. Ship `1.0.0-rc.N` to the `next`
channel; the handshake rejects a v1 daemon or plugin. Migrate
`~/.config/chauffeur`, or read v1's layout, without losing personal rules. When
every parity row is done, keep `v1` as a hotfix branch, merge v2 into `main`
and tag `1.0.0`.

Verification: packaging dry run and smoke test on Linux and macOS; Homebrew
`install`, `test`, `audit` and `style`; upgrade from the current v1 release on
a machine with personal rules; rc release-gate runs from the bench.

## Quality gates

- Pinned nightly, deny-level strict Clippy, formatting and stable MSRV check.
- Miri over core and runtime scenarios; Kani proofs are small and each names
  one invariant.
- Real boundary checks for browser WASM, the daemon socket, each `navigate` binding, provider HTTP and
  live Jev. Unit tests supplement these checks; they do not replace them.
- Verify at each slice before proceeding. Keep functions under 70 non-blank,
  non-comment lines, use no production dependency without approval, and keep
  core dependency-free.
- Full workspace tests, clippy, Miri, Kani, docs and deny run before committing
  the completed slice. The v2 branch is pushed after the agreed review.

## Open questions

| Question | Recommended default |
|---|---|
| Event observation API | Core returns transition events; runtime offers an observer callback, and the daemon streams them to the caller from a bounded log. Judges are the only user code core calls |
| Plan outcome types | Native plans choose `O: Clone + Eq`; protocol outcomes are a string, an integer or a list of candidate names, with `Failed` a distinct machine result |
| Slice 2 dependencies | `serde` and `serde_json` in protocol. The daemon's socket and the runtime use `std` only; core stays dependency-free |
| Gate source semantics | Any/All over preceding step IDs; skipped is terminal non-match; failures match only when named |
| Rule-file trust | A project's `.chauffeur/rules/` may use facts and System One; System Two needs the user's `~/.config/chauffeur` to allow it for that project. Integration rules sit in the user's folder and are trusted as the user's |
| Rule reload | The daemon polls rule folders' modification times with `std`; a file watcher needs a dependency |
| o11y backend | Begin with a small event-consumer crate; choose a tracing/logging dependency only when implementing it |
