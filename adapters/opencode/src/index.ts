import { Plugin } from "@opencode/plugin/effect"
import { Effect } from "effect"
import { installAskTool } from "./ask-tool.js"
import { Daemon } from "./daemon.js"
import { installExposure } from "./exposure.js"
import { installGate } from "./gate.js"
import { installHandoffSkills } from "./handoff.js"
import { Host } from "./host.js"
import { installIdle } from "./idle.js"
import { installModelRouter } from "./model-router.js"
import { installPermission } from "./permission.js"
import { installRulebooks } from "./rulebooks.js"
import { installRemoteSkills } from "./remote-skills.js"
import { installRenewal } from "./renewal.js"
import { claimSkillLoading, hostLoadsSkills } from "./skills.js"
import { installToolDescriptions } from "./tool-descriptions.js"
import { followDaemonTexts, Texts } from "./texts.js"
import { installToolResults } from "./tool-results.js"
import { installWarming } from "./warming.js"

export { ChauffeurRpc } from "./gate.js"

/** Every capability's hooks live in the plugin scope, so unloading releases them together. */
const capabilities = Effect.gen(function* () {
  if (!hostLoadsSkills()) yield* claimSkillLoading
  yield* installModelRouter

  const warming = yield* installWarming
  const renewal = yield* installRenewal(warming)
  const exposure = yield* installExposure(renewal)

  yield* installPermission
  yield* installToolResults(exposure)
  yield* installAskTool(exposure)
  yield* installToolDescriptions
  yield* installHandoffSkills
  yield* installRemoteSkills
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
