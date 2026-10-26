# Chauffeur v2

A clean-room rebuild of Chauffeur as a generic judgment runtime. Chauffeur
releases are built from [`main`](https://github.com/StevenJPx2/chauffeur/tree/main);
this branch ships nothing until cutover.

Callers such as sourcefed, opencode-history and host adapters send a question
with its context through `chauffeur.navigate`, using the TypeScript client, the
`chauffeur` CLI or a host adapter. Rule files decide how to answer it. Rules
come from Chauffeur, integrations, the user and projects, and hold data only.

A rule names its **judges**, its **asks** and its **strategy**: a list of
steps, each waiting on earlier ones with `after`. Judges come in four tiers,
deterministic facts, System One, System Two and a person, and all take the
same question and give the same answer. Models, such as Jev, GLiDE or a local
model, are configuration.

```jsonc
"judges":   { "quick": { "tier": "system-one", "model": "jev", "confidence": 0.7 },
              "careful": { "tier": "system-one", "model": "glide", "confidence": 0.7 } },
"strategy": [
  { "id": "fast", "judge": "quick",   "ask": "needs-agent" },
  { "id": "slow", "judge": "careful", "ask": "needs-agent", "after": [{ "step": "fast", "when": ["unsure", "failed"] }] }
]
```

`chauffeur strategy show` draws a strategy as a tree.

## Status

| Slice | Scope | State |
|---|---|---|
| 0 | Browser playground: Rust compiled to WebAssembly | Built |
| 1 | `crates/core`: plans, `after`, judges, state machine, events | Built with gates; rewrite to `after` planned |
| 2 | `chauffeur.navigate`: protocol, judge tiers and models, rule files, runtime, daemon socket, through the client, CLI and OpenCode | Planned |
| 3 | System One models: Jev, SubQ and GLiDE | Planned |
| 4 | The first System Two models | Planned |
| 5 | Behavior plugins and OpenCode adapter, at parity with `main` | Planned |
| 6 | Release and cutover | Planned |

[`docs/plans/pluggability.md`](docs/plans/pluggability.md) is the plan: the
architecture, each slice, and the parity table tracking `main`'s features.
[`docs/audits/architecture.md`](docs/audits/architecture.md) records the v1
findings it answers.

## Layout

| Path | Contents |
|---|---|
| `crates/core` | Plans, judges and the state machine; no dependencies |
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
