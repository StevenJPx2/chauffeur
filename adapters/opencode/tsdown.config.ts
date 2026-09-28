import { defineConfig } from "tsdown"

/**
 * One self-contained ESM file for OpenCode to load. Dependencies are bundled
 * so the installed plugin never resolves packages from the host's
 * node_modules, and a build is the only way a change reaches a running host.
 */
export default defineConfig({
  entry: ["src/index.ts"],
  format: ["esm"],
  platform: "node",
  target: "node22",
  outDir: "dist",
  clean: true,
  dts: false,
  // A dependency not listed here fails the build instead of slipping in.
  deps: { alwaysBundle: [/.*/], onlyBundle: ["@opencode/plugin", "@opencode/schema", "effect"] },
})
