import { Plugin } from "@opencode/plugin"
import { DaemonBridge } from "./daemon.js"
import { installExposure } from "./exposure.js"
import { installGate } from "./gate.js"
import { installIdle } from "./idle.js"
import { installModelRouter } from "./model-router.js"
import { installPermission } from "./permission.js"
import { claimSkillLoading } from "./skills.js"
import { installToolResults } from "./tool-results.js"

export { ChauffeurRpc } from "./gate.js"

export default Plugin.define({
  id: "chauffeur",
  async setup(ctx) {
    const daemon = new DaemonBridge()

    try {
      await daemon.start()
    } catch (error) {
      // Each capability applies its own failure posture while the daemon is down.
      console.error(`[chauffeur] daemon unavailable: ${error instanceof Error ? error.message : String(error)}`)
    }

    const disposeSkillLoading = await claimSkillLoading(ctx)
    const disposeModelRouter = await installModelRouter(ctx, daemon)
    const exposure = await installExposure(ctx, daemon)
    const disposePermission = await installPermission(ctx, daemon)
    const disposeToolResults = await installToolResults(ctx, daemon, exposure)
    const disposeGate = await installGate(ctx, daemon)
    const disposeIdle = installIdle(ctx, daemon)

    return async () => {
      disposeIdle()
      await disposeGate()
      await disposeSkillLoading()
      await disposePermission()
      await disposeToolResults()
      await disposeModelRouter()
      await exposure.dispose()
    }
  },
})
