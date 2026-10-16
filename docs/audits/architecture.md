# Architecture audit

## Scope and baseline

Audited commit: `73bc29e` on `origin/main`, in an isolated worktree. Uncommitted
experiments and local checkout changes are excluded. This is a dependency,
lifecycle and restructure-readiness audit; it is not a complete security review.

The workspace has 18 Rust packages:

- seven capabilities
- three judge packages
- three provider and integration packages
- core, daemon, CLI, MCP and the benchmark

OpenCode is the one implemented host adapter.

| Executed check | Result |
|---|---|
| `cargo fmt --all -- --check` | Pass |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Pass |
| `cargo test --workspace --locked` | 378 passed; 3 ignored |
| `cargo check -p chauffeur-core --no-default-features --locked` | Pass |
| OpenCode `npm ci` | Zero reported vulnerabilities; deprecation warnings for transitive `glob` and `node-domexception` |
| OpenCode `npm run check` | Types and lint pass; 69 tests pass; changed-file Fallow audit covers zero changed files |
| OpenCode full `fallow` analysis | Zero dead-code, cycle, clone and health-threshold findings across 46 files and 665 functions |
| `node --test scripts/release.test.mjs` | 5 passed |
| `cargo run -p chauffeur-bench --locked -- verify` | All 15 fixtures distinguish unsolved and solved work |

The ignored tests cover real-daemon startup and live provider calls. No paid
inference or credential refresh ran. Fallow's coverage is a static estimate.
The benchmark verifier checks fixtures, not agent quality.

## Findings

### A1. High: the engine is an agent application, not a generic pipeline runner

- `crates/core/src/contracts/signal.rs:141-287` defines skills, Code Mode,
  permission requests, todos, rulebooks, model switching and integration events.
- `crates/core/src/contracts/effect.rs:34-74` defines agent-facing actions.
- `crates/core/src/engine.rs:303-405` records agent history and applies
  permission vetoes before any capability; `632-665` overrides permission
  results and learns safety policy.
- `crates/core/src/judge/judged.rs:15-35` couples the judgment adapter to
  `Signal`, `Situation` and `Effect`.

A new domain can compose `Judge<T>`, but must adopt agent concepts to use the
hosted engine. Moving folders alone does not produce a reusable core.

### A2. High: native continuations cannot cross an RPC boundary

`crates/core/src/judge/mod.rs:21-33` stores each continuation as a Rust
`FnOnce`. `then`, `all` and the strategies are native primitives, not a wire
format. The System One question and answer types have no serde contract.

The daemon serves health, signal, rulebooks, skills, texts and shutdown
(`crates/daemon/src/lib.rs:252-269`). It has no pipeline submission, plugin
registration, event subscription or action brokerage. A pair of classify and
complete endpoints would expose single calls, not composition.

### A3. High: the queue limit does not bound admitted waiting work

`crates/daemon/src/scheduler.rs:152-155` checks the `queued` counter, but
`366-368` decrements it when the job moves into the unbounded `waiting` deque.
New arrivals refill the inbox while earlier jobs remain waiting. Deadlines limit
how long a job waits, not how many are retained.

Reproduction through the public `EngineHandle` API, with one agent, a
deliberately slow three-second classifier and no network:

- 81 requests submitted, three milliseconds apart.
- Observed `submitted=81 still_waiting=81 provider_calls=1`.

None of the excess was rejected. External admission needs a permit held across
inbox, waiting and execution, released on every terminal path.

### A4. High: SourceFed I/O blocks the coordinator

- `capabilities/monitors/src/lib.rs:189-243` lists monitors while planning, and
  `138-160` creates them while acting.
- `capabilities/event-gate/src/lib.rs:133-149` lists monitors while composing
  its question.
- These calls block on `plugins/sourcefed/src/lib.rs:49-105`, which has a
  two-second timeout.
- Only inference runs on workers; planning and acting run on the coordinator
  (`crates/daemon/src/scheduler.rs:483-512`, `664-695`).

With a loopback SourceFed stand-in that delays its reply, an unrelated
permission request needing no inference waited **2001 ms**. Reserved inference
capacity does not protect against plugin I/O on the coordinator.

### A5. High: cancellation does not cover domain state or side effects

`crates/core/src/judge/judged.rs:94-98` and `124-132` call `act` as each
capability settles, while other capabilities can still be pending. An acted
capability can mutate state or create a SourceFed monitor before the run
returns. `engine.rs:408-421` forgets outstanding work but cannot undo actions.
The scheduler snapshots state only for model errors (`scheduler.rs:340-344`,
`697-709`).

External execution needs observation revisions and explicit commit or delivery.
A caller disconnecting is not rollback and does not prove nothing happened.

### A6. Medium: daemon assembly owns features and concrete providers

- `crates/daemon/src/engine.rs:164-230` constructs every capability and
  concrete provider.
- `reasoning.rs:262-281` imports Codex authentication and builds its backend.
- `judge.rs:58-107` selects Jev or SubQ.
- The HTTP state owns rulebooks, host wording, skill sources and skill ranking
  (`lib.rs:43-58`), and the router has a SourceFed-specific route.

A reusable runtime needs injected implementations. The shipped application
still needs one explicit composition root that selects the default plugins.

### A7. Medium: provider metadata depends on a behavior crate

The Anthropic and OpenAI crates import the model-router crate, whose
`src/provider.rs` owns `Provider`, `TierTable` and the tiers. `Backend` lives
in core. Anthropic supplies metadata only; OpenAI supplies metadata and Codex
execution. A tier rating is not evidence that a model can be executed.

The shared routing-metadata contract should leave the router without moving
routing policy into core or rewriting the working backends.

### A8. Medium: embedding transport is mixed with skill policy

`crates/daemon/src/ranking.rs:62-92`, `198-216` and `251-279` validate the
loopback endpoint and carry embedding requests. The same type keeps per-agent
prompts, skill catalogs, vector caches and ranking (`40-49`, `95-196`,
`282-299`). It calls a separately managed local server.

Only transport and model/response validation are reusable service code; skill
ranking and its cache belong to skill exposure. Responses are decoded without a
byte or dimension ceiling, which external callers would need.

### A9. Medium: host glue, feature activation and wire contracts are intertwined

The adapter does essential host work, such as rejecting stale model switches
(`adapters/opencode/src/model-router.ts:264-283`). It also:

- installs every feature at startup (`index.ts:22-37`);
- denies the host's skill tool (`skills.ts:59-75`);
- hand-maintains Rust-shaped signals and effects (`protocol.ts`).

Core's default `client` feature pulls in HTTP, and `client.rs` carries a
version-compatibility retry.

Host-level cancellation, permission enforcement, request-local tools and prompt
cache behavior must survive the split. Feature activation and wire generation
move to their owners; adapter files are not moved wholesale into an SDK.

### A10. Medium: configuration, state and packaging cross directory boundaries

`skills/README.md:8-25` describes seven directories with unrelated roles:

- permission contracts
- rules
- rulebooks
- safety policy
- tunable config
- hand-over skills
- Chauffeur's own skill

Compiled defaults use relative `include_str!` paths from core, capabilities and
providers into `skills/`. `crates/daemon/src/engine.rs:101-117` reloads by
rebuilding the engine and restoring its state, keyed per agent and per
capability.

`scripts/package.mjs:22-55`, `scripts/homebrew.mjs:74-117`, CLI setup and the
adapter's daemon launch assume one binary, one OpenCode bundle, the shared
`skills/` tree and matching versions. Moves must update packaging tests, asset
resolution, hot reload and state migration together. External plugins need
protocol compatibility independent of the bundled adapter's version.

### A11. High before external access: one token authorizes everything

`crates/daemon/src/lib.rs:350-359` accepts any call when no token is set; one
token authorizes every method, including shutdown. There is no per-plugin
authority, event scope or quota. Redaction and learned state live inside the
agent engine, so a standalone inference path could bypass them.

A caller-supplied plugin ID must not become authority. Inference, observations,
host actions and administration need separate grants. Native and external
plugins remain trusted code, not sandboxed code.

## Foundations to preserve

- Judgment composition and the explicit distinction between unsure and failed.
- Provider-neutral inference interfaces and schema validation.
- Native provider implementations and shared Codex login state.
- Per-run state and rejection of late results.
- Reserved interactive capacity and host deadlines.
- Hot reload that keeps the last valid configuration.
- Conservative permission behavior and its safety regressions.
- Skill-source caching, atomic materialization and immutable revision folders.
- The adapter's handling of host denials, compaction, inheritance and stale effects.

## Reproductions and limits

The A3 and A4 reproductions are scratch binaries outside the repository. They
use the unmodified workspace crates and its lockfile, and touch no repository
file or running service. Turning both into regression tests is the plan's first
step.

Unmeasured:

- CPU and memory cost of RPC plugins
- release-build performance
- external-plugin compatibility
- any other host adapter

The [restructure plan](../plans/pluggability.md) defines the target and its gates.
