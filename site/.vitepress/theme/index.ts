import DefaultTheme from "vitepress/theme"
import { useData, useRoute } from "vitepress"
import { nextTick, onMounted, watch } from "vue"

/**
 * Draws every `pre.mermaid` block on the page. Each keeps its source, so a
 * switch between light and dark draws it again in the new theme.
 */
async function draw(dark: boolean) {
  const blocks = [...document.querySelectorAll<HTMLElement>("pre.mermaid")]
  if (blocks.length === 0) return
  for (const block of blocks) {
    block.dataset.source ??= block.textContent ?? ""
    block.textContent = block.dataset.source
    block.removeAttribute("data-processed")
  }
  const { default: mermaid } = await import("mermaid")
  mermaid.initialize({ startOnLoad: false, theme: dark ? "dark" : "default" })
  await mermaid.run({ nodes: blocks })
}

export default {
  extends: DefaultTheme,
  setup() {
    const route = useRoute()
    const { isDark } = useData()
    const redraw = () => nextTick(() => draw(isDark.value))
    onMounted(redraw)
    watch(() => route.path, redraw)
    watch(isDark, redraw)
  },
}
