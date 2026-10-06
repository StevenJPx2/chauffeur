import { Plugin } from "@opencode/plugin/effect"
import { Effect } from "effect"
import { installAskTool } from "./ask-tool.js"
import { Daemon } from "./daemon.js"
import { installExposure } from "./exposure.js"
import { installGate } from "./gate.js"
import { Host } from "./host.js"
import { installIdle } from "./idle.js"
import { installModelRouter } from "./model-router.js"
import { installPermission } from "./permission.js"
import { installRulebooks } from "./rulebooks.js"
import { claimSkillLoading, hostLoadsSkills } from "./skills.js"
import { installSubagentGuidance } from "./subagents.js"
import { followDaemonTexts, Texts } from "./texts.js"
import { installTodos } from "./todos.js"
import { installToolResults } from "./tool-results.js"

export { ChauffeurRpc } from "./gate.js"

/** Every capability's hooks live in the plugin scope, so unloading releases them together. */
const capabilities = Effect.gen(function* () {
  if (!hostLoadsSkills()) yield* claimSkillLoading
  yield* installModelRouter

  const exposure = yield* installExposure

  yield* installPermission
  yield* installToolResults(exposure)
  yield* installAskTool(exposure)
  yield* installTodos
  yield* installSubagentGuidance
  yield* installGate
  yield* installIdle
  yield* installRulebooks
})

/** The daemon's wording first, so every capability registers with it. */
const plugin = Effect.gen(function* () {
  const texts = yield* followDaemonTexts

  yield* capabilities.pipe(Effect.provideService(Texts, texts))
})

export default Plugin.define({
  id: "chauffeur",
  effect: (ctx) =>
    Daemon.connect.pipe(Effect.flatMap((daemon) =>
      plugin.pipe(Effect.provideService(Host, ctx), Effect.provideService(Daemon, daemon)))),
})
