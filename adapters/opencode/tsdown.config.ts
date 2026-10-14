import { defineConfig } from "tsdown"

/**
 * Bundle the adapter, not the host runtime. OpenCode supplies Effect and its
 * plugin SDK: a second bundled Effect runtime cannot safely consume the host's
 * streams or share its scoped fibers, even when the versions match.
 */
export default defineConfig({
  entry: ["src/index.ts"],
  format: ["esm"],
  platform: "node",
  target: "node22",
  outDir: "dist",
  clean: true,
  dts: false,
  deps: { neverBundle: ["effect", /^@opencode\//] },
})
