# Chauffeur v2

A clean-room rebuild of Chauffeur as a judgment engine. Releases are built from
[`main`](https://github.com/StevenJPx2/chauffeur/tree/main); this branch ships
nothing until cutover.

## What exists

- **`crates/core`**: runs bounded judgment plans, with no dependencies.
  - A plan is an ordered list of named steps. Each step names the judge that
    answers it, and gates on earlier steps decide whether it runs.
  - A judge is anything that answers a step, whether a rule, a model or a person.
    A `Panel` maps names to judges, and `Plan::run` asks each judge its share of
    a round in one call.
  - The round-by-round API returns events as data: `Started`, `RoundReady`,
    `StepSettled`, `StepSkipped` and `Finished`.
  - Kani proofs show that every plan finishes within one round per step.
- **`playground`**: a browser page that drives core through WebAssembly. Its
  State machine tab steps through example plans with answers picked by hand.

[`docs/plans/pluggability.md`](docs/plans/pluggability.md) is the plan for the
rest: strategies with `after`, facts, models, `chauffeur.navigate`, plugins
and adapters.

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
