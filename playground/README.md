# Playground

A page for trying Chauffeur's Rust code by hand. The Rust side (`wasm/`) is
compiled to WebAssembly and loaded by a Vite page. Each feature under test is
its own tab.

- **UI:** [petite-vue](https://github.com/vuejs/petite-vue) for reactivity,
  [Web Awesome](https://webawesome.com) for components.
- **Run:** `bun install`, then `bun run dev`. Editing Rust in `wasm/` or
  `../crates/` rebuilds it and reloads the page.
- **Tabs:** each slice adds the tab for its feature. *Question* settles a
  question's probabilities under a confidence with `chauffeur-judges`, as the
  daemon does: an outcome, `unsure`, or `failed` for a reply that is not an
  answer.
- **Check:** `bun run check` builds the release module and the page, and
  type-checks.

## Adding a feature tab

1. Export the Rust function from `wasm/src/lib.rs` with `#[wasm_bindgen]`.
2. Add `src/features/<name>.ts` exporting a `Feature`: its `view()` returns the
   tab's reactive state and names its template.
3. Add the `<template id="<name>">` to `index.html`.
4. Add the feature to `features` in `src/main.ts`, and import any Web Awesome
   component the template uses.
