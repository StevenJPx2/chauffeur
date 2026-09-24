import { Plugin } from "@opencode/plugin/effect"
import { Effect } from "effect"
import { Daemon } from "./daemon.js"
import { installExposure } from "./exposure.js"
import { installGate } from "./gate.js"
import { Host } from "./host.js"
import { installIdle } from "./idle.js"
import { installModelRouter } from "./model-router.js"
import { installPermission } from "./permission.js"
import { claimSkillLoading } from "./skills.js"
import { installToolResults } from "./tool-results.js"

export { ChauffeurRpc } from "./gate.js"

/** Every capability's hooks live in the plugin scope, so unloading releases them together. */
const capabilities = Effect.gen(function* () {
  yield* claimSkillLoading
  yield* installModelRouter

  const exposure = yield* installExposure

  yield* installPermission
  yield* installToolResults(exposure)
  yield* installGate
  yield* installIdle
})

export default Plugin.define({
  id: "chauffeur",
  effect: (ctx) =>
    Daemon.connect.pipe(Effect.flatMap((daemon) =>
      capabilities.pipe(Effect.provideService(Host, ctx), Effect.provideService(Daemon, daemon)))),
})
