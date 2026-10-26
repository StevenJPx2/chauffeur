# Chauffeur

Chauffeur judges what your coding agent is doing and steers it. It watches
the harness's events, asks fast and careful judges the right questions, and
acts through the harness: attaching skills, compacting a cold session,
stopping a destructive command or holding back noise.

## Features

- **Strategies**
  - A strategy is a list of steps; each step asks one judge one question.
  - `after` makes a step wait for earlier steps and run only when their
    outcomes match, such as escalating from Jev to GLiDE to a reasoning model
    when the first is unsure.
  - `for_each` asks about many candidates in one round, and `select` ranks,
    filters or votes on the answers.
  - Every strategy has a known maximum of rounds and model calls, and is
    proven to finish.
- **Judges**
  - Four tiers answer the same questions: facts (code), System One (fast
    classifiers), System Two (reasoning models) and a person.
  - `confidence` sets how sure a judge must be; below it, the answer is
    `unsure`.
- **Questions and results**
  - `noul` (yes or no), `choice` and `score` build questions; `question()`
    builds your own kinds.
  - A result is an answer, with its outcome, every outcome's probability and a
    level on the scale, or a failure with its reason. A failure never reads as
    `no`.
  - Outcomes are typed in TypeScript from the question that produced them.
- **Models as configuration**
  - `models.json` registers models by API format: `systemone` (Jev, SubQ,
    GLiDE), `openai` (including local servers such as Ollama) and `anthropic`.
  - Each tier lists its models in fallback order; a missing model falls back
    and the run says so.
  - `placeholder` models stand in with recorded or fixed answers, for tests
    and tuning.
  - Keys stay inside the engine; rules, plugins and logs never see them.
- **Facts**
  - JavaScript modules that answer without a model, run in a QuickJS sandbox
    with memory and time limits.
  - Shipped facts: `match`, `select`, `bm25` and `embedding`.
  - Write your own in `~/.config/chauffeur/facts/`, with read-only access to
    files, SQLite and local services. A project's facts are pure functions.
- **Rule files**
  - JSON that names an event, its judges, its questions, its strategy and what
    to do with the outcome. No code needed.
  - Rules come from Chauffeur, integrations, you and your projects. A project's
    rules use System Two only where you allow it.
- **Plugins**
  - TypeScript programs that hook events with `ctx.on` (never blocks) or
    `ctx.before` (holds the harness within a budget, then asks you).
  - They act through every control the harness offers, named by resource and
    verb: `session.compact`, `session.update`, `message.append` and more.
  - They load when first needed, stop when idle, and restart if they crash.
  - Included: skills, context renewal, session warming and a guard against
    irreversible commands.
- **Adapters**
  - One per harness, turning its hooks into standard events such as
    `prompt.sent` and `tool.before`, and carrying out actions after checking
    they are still current.
  - Available for OpenCode.
- **`navigate`**
  - Any program can send an event with its context and get back intents, from
    TypeScript (`@fdcn/chauffeur/client`), the `chauffeur navigate` CLI or
    Rust.
- **Tools**
  - `chauffeur strategy show` draws a rule's strategy as a tree, with the
    models it will use and its maximum rounds and calls.
  - The playground steps through strategies in the browser.
- **Safety**
  - The daemon listens only on a Unix socket in a directory only you can open.
    There are no ports and no tokens.
  - Evidence is redacted before every model call.
  - Your harness's own denials always win.

## Examples

A rule that screens with Jev and escalates to GLiDE when Jev is unsure:

```jsonc
{
  "on": "integration_event",
  "judges": {
    "quick":   { "tier": "system-one", "model": "jev",   "confidence": 0.7 },
    "careful": { "tier": "system-one", "model": "glide", "confidence": 0.7 }
  },
  "asks": { "needs-agent": { "outcomes": ["yes", "no"], "instructions": "Does this event ask the agent to do something?" } },
  "strategy": [
    { "id": "fast", "judge": "quick",   "ask": "needs-agent" },
    { "id": "slow", "judge": "careful", "ask": "needs-agent", "after": [{ "step": "fast", "when": ["unsure", "failed"] }] }
  ],
  "then": [{ "if": { "fast": "no" }, "withhold": "event" }, { "if": { "slow": "no" }, "withhold": "event" }]
}
```

A plugin that asks before a risky shell command runs:

```ts
import { Plugin } from "@fdcn/chauffeur/plugin"
import { ask, judge, noul } from "@fdcn/chauffeur/judge"

const guard = judge({ tier: "system-one", confidence: { yes: 0.6, no: 0.95 } })

export default Plugin.define({
  id: "guard",
  setup(ctx) {
    ctx.before("tool.before", async (e) => {
      if (e.tool !== "shell") return
      const harmful = await ask(noul("Could this command cause irreversible harm?"), { judge: guard, evidence: { command: e.input.command } })
      if (!harmful.ok || harmful.outcome !== "no") e.askUser("Chauffeur isn't sure this is safe.")
    }, { budget: 2_000, onTimeout: "ask" })
  },
})
```

## Documentation

[`docs/plans/pluggability.md`](docs/plans/pluggability.md) describes the
architecture: strategies, judges, facts, rule files, plugins, adapters and the
`navigate` protocol.

## Develop

The toolchain is pinned in `rust-toolchain.toml`, with Miri and the
WebAssembly target.

```sh
cargo test --workspace
cargo clippy --workspace --all-targets --locked
cargo miri test -p chauffeur-core
cargo kani -p chauffeur-core --harness <name>   # harnesses in crates/core/src/proofs.rs
```

The playground needs Bun and `wasm-pack`:

```sh
cd playground
bun install
bun run dev     # rebuilds Rust on change and reloads the page
bun run check   # release build and type check
```

## License

MIT. See [LICENSE](LICENSE).
