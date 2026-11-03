import { existsSync, readFileSync, readdirSync, statSync } from "node:fs"
import { dirname, join, relative, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { defineConfig, type DefaultTheme } from "vitepress"

const REPOSITORY = "https://github.com/StevenJPx2/chauffeur"
const BRANCH = "v2"
const SITE = resolve(dirname(fileURLToPath(import.meta.url)), "..")
const ROOT = resolve(SITE, "..")
const SKIPPED = new Set([".git", ".github", ".vitepress", "node_modules", "target", "dist", "wasm"])

/** Every Markdown file in the repository, relative to its root. */
function pages(folder = ROOT): string[] {
  return readdirSync(folder, { withFileTypes: true })
    .filter((entry) => !SKIPPED.has(entry.name) && !entry.name.startsWith("mutants.out"))
    .flatMap((entry) => {
      const path = join(folder, entry.name)
      if (entry.isDirectory()) return pages(path)
      return entry.name.endsWith(".md") ? [relative(ROOT, path)] : []
    })
    .toSorted()
}

/** A folder's `README.md` is its index page, so `crates/core/src/plan/` links work. */
const rewrite = (page: string) => page.replace(/(^|\/)README\.md$/, "$1index.md")

function title(page: string): string {
  return readFileSync(join(ROOT, page), "utf8").match(/^# (.+)$/m)?.[1].replaceAll("`", "") ?? page
}

function link(page: string): string {
  return `/${rewrite(page).replace(/(^|\/)index\.md$/, "$1").replace(/\.md$/, "")}`
}

/** `crates/judges/src/tiers/facts/README.md` reads as `judges · tiers · facts`. */
function moduleName(page: string): string {
  const [crate, , ...module] = dirname(page).split("/").slice(1)
  return [crate, ...module].join(" · ")
}

function sidebar(all: string[]): DefaultTheme.SidebarItem[] {
  const under = (prefix: string) => all.filter((page) => page.startsWith(prefix))
  const item = (page: string, text = title(page)) => ({ text, link: link(page) })
  const crates = under("crates/").filter((page) => page !== "crates/README.md")
  return [
    { text: "Overview", items: [item("README.md", "Chauffeur")] },
    { text: "Plans", items: under("docs/plans/").map((page) => item(page)) },
    { text: "Audits", items: under("docs/audits/").map((page) => item(page)) },
    {
      text: "Crates",
      items: [item("crates/README.md", "All crates"), ...crates.map((page) => item(page, moduleName(page)))],
    },
    { text: "Tools", items: [...under("playground/"), ...under("site/")].map((page) => item(page)) },
  ]
}

/**
 * A relative link to a file the site does not render, such as a `.rs` file
 * or a folder without a README, goes to that path on GitHub instead.
 */
function sourceLink(href: string, from: string): string {
  if (/^([a-z]+:|#|\/)/i.test(href)) return href
  const [path, hash] = href.split("#")
  const target = resolve(dirname(from), decodeURI(path))
  const rendered = target.endsWith(".md") || existsSync(join(target, "README.md"))
  if (rendered || !target.startsWith(ROOT) || !existsSync(target)) return href
  const kind = statSync(target).isDirectory() ? "tree" : "blob"
  const anchor = hash === undefined ? "" : `#${hash}`
  return `${REPOSITORY}/${kind}/${BRANCH}/${relative(ROOT, target)}${anchor}`
}

const all = pages()

export default defineConfig({
  title: "Chauffeur",
  description: "A judgment engine for coding agents",
  srcDir: ROOT,
  srcExclude: ["**/node_modules/**", "target/**", "mutants.out*/**"],
  rewrites: Object.fromEntries(all.map((page) => [page, rewrite(page)])),
  cleanUrls: true,
  // Pages live outside this folder, so their imports resolve from its node_modules.
  vite: {
    resolve: { alias: [{ find: /^vue(?=$|\/)/, replacement: join(SITE, "node_modules/vue") }] },
    // Mermaid's own chunks reach 674 kB; they load only on a page with a diagram.
    build: { chunkSizeWarningLimit: 700 },
  },
  lastUpdated: true,
  themeConfig: {
    nav: [
      { text: "Roadmap", link: "/docs/plans/pluggability#goal" },
      { text: "Slices", link: "/docs/plans/pluggability#sequence-and-verification" },
      { text: "Crates", link: "/crates/" },
    ],
    sidebar: sidebar(all),
    outline: [2, 3],
    search: { provider: "local" },
    socialLinks: [{ icon: "github", link: `${REPOSITORY}/tree/${BRANCH}` }],
    editLink: { pattern: `${REPOSITORY}/edit/${BRANCH}/:path` },
  },
  markdown: {
    config(md) {
      const fence = md.renderer.rules.fence
      md.renderer.rules.fence = (tokens, at, options, env, self) => {
        const token = tokens[at]
        if (token.info.trim() !== "mermaid") return fence?.(tokens, at, options, env, self) ?? ""
        // v-pre keeps Vue from reading {{ }} in a diagram as a template.
        return `<pre class="mermaid" v-pre>${md.utils.escapeHtml(token.content)}</pre>`
      }
      md.core.ruler.push("source-links", (state) => {
        const from = state.env.path as string | undefined
        if (!from) return
        for (const token of state.tokens.flatMap((block) => block.children ?? [])) {
          const href = token.type === "link_open" ? token.attrGet("href") : null
          if (href) token.attrSet("href", sourceLink(href, from))
        }
      })
    },
  },
})
