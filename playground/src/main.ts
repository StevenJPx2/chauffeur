import "@awesome.me/webawesome/dist/styles/webawesome.css"
import "@awesome.me/webawesome/dist/components/badge/badge.js"
import "@awesome.me/webawesome/dist/components/callout/callout.js"
import "@awesome.me/webawesome/dist/components/card/card.js"
import "@awesome.me/webawesome/dist/components/option/option.js"
import "@awesome.me/webawesome/dist/components/select/select.js"
import "@awesome.me/webawesome/dist/components/tab-group/tab-group.js"
import "@awesome.me/webawesome/dist/components/tab-panel/tab-panel.js"
import "@awesome.me/webawesome/dist/components/tab/tab.js"
import "@awesome.me/webawesome/dist/components/textarea/textarea.js"
import "./style.css"
import { createApp, reactive } from "petite-vue"
import type { Feature } from "./feature.ts"
import { question } from "./features/question.ts"
import init, { version } from "./wasm/chauffeur_playground.js"

/** Every feature under test, one tab each, in tab order. */
const features: ReadonlyArray<Feature> = [question]

/** The tab the URL names, or the first one. */
function named(): string {
  const id = location.hash.slice(1)

  return features.some((feature) => feature.id === id) ? id : (features[0]?.id ?? "")
}

await init()

const page = reactive({
  features,
  version: version(),
  selected: named(),
  /** Keep the URL on the shown tab, so a reload returns to it. */
  show(id: string): void {
    page.selected = id
    history.replaceState(null, "", `#${id}`)
  },
})

window.addEventListener("hashchange", () => {
  page.selected = named()
})

createApp(page).mount("main")
