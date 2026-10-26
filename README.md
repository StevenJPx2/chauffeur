# Chauffeur v2

A clean-room rebuild of Chauffeur as a generic judgment runtime. Chauffeur
releases are built from [`main`](https://github.com/StevenJPx2/chauffeur/tree/main);
this branch ships nothing until cutover.

Core runs bounded **plans**: named steps joined by gates, each answered by a
**judge**. Judges come in four tiers, deterministic facts, System One, System
Two and a person, and all take the same question and give the same answer.
Model providers, such as Jev or a local model, are configuration.

Callers such as sourcefed, opencode-history and host adapters send a question
with its context through `chauffeur.navigate`, using the TypeScript client, the
`chauffeur` CLI or a host adapter. Rule files decide how to answer it: when to
ask, the plan, and what to do with the outcome. Rules come from Chauffeur,
integrations, the user and projects, and hold data only.

```rust
let plan = ask("screen", "rule", evidence)
    .then_if_any(Outcome::Value(Unsure), ask("review", "model", evidence));
let run = plan.run(&Panel::new().with("rule", &rule).with("model", &model))?;
```

## Status

| Slice | Scope | State |
|---|---|---|
| 0 | Browser playground: Rust compiled to WebAssembly | Built |
| 1 | `crates/core`: plans, gates, judges, state machine, events | Built and reviewed |
| 2 | `chauffeur.navigate`: protocol, judge tiers and providers, rule files, runtime, daemon socket, through the client, CLI and OpenCode | Planned |
| 3 | Jev as the first System One provider | Planned |
| 4 | The first System Two providers | Planned |
| 5 | Behavior plugins and OpenCode adapter, at parity with `main` | Planned |
| 6 | Release and cutover | Planned |

[`docs/plans/pluggability.md`](docs/plans/pluggability.md) is the plan: the
architecture, each slice, and the parity table tracking `main`'s features.
[`docs/audits/architecture.md`](docs/audits/architecture.md) records the v1
findings it answers.

## Layout

| Path | Contents |
|---|---|
| `crates/core` | Plans, gates, judges and the state machine; no dependencies |
| `playground` | Vite page driving core through WebAssembly; see its [README](playground/README.md) |
| `docs` | The plan and the v1 audit |

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
