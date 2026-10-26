# Playground

A page for trying Chauffeur's Rust code by hand. The Rust side (`wasm/`) is
compiled to WebAssembly and loaded by a Vite page. Each feature under test is
its own tab.

- **UI:** [petite-vue](https://github.com/vuejs/petite-vue) for reactivity,
  [Web Awesome](https://webawesome.com) for components.
- **Run:** `bun install`, then `bun run dev`. Editing Rust in `wasm/` or
  `../crates/` rebuilds it and reloads the page.
- **Tabs:** *State machine* runs a JSON plan through `chauffeur-core` one round
  at a time. Each step names its judge: `person` steps are answered by hand,
  and `keywords` steps by a deterministic rule judge in Rust.
- **Check:** `bun run check` builds the release module and the page, and
  type-checks.

## Adding a feature tab

1. Export the Rust function from `wasm/src/lib.rs` with `#[wasm_bindgen]`.
2. Add `src/features/<name>.ts` exporting a `Feature`: its `view()` returns the
   tab's reactive state and names its template.
3. Add the `<template id="<name>">` to `index.html`.
4. Add the feature to `features` in `src/main.ts`, and import any Web Awesome
   component the template uses.
