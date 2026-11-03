# Site

The documentation site: every Markdown file in the repository, rendered with
[VitePress](https://vitepress.dev), with Mermaid diagrams drawn and local
search. The plan's slice table is the roadmap.

| Path                        | Holds                                                                                                                    |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `.vitepress/config.mts`     | Finds the pages, builds the sidebar, makes each folder's `README.md` its index, and points links to code files at GitHub |
| `.vitepress/theme/index.ts` | Draws `mermaid` blocks, again when the page or the colour scheme changes                                                 |

- **Run:** `bun install`, then `bun run dev`.
- **Build:** `bun run build` writes `.vitepress/dist/`.
- **Format:** `bun run format` runs Prettier over every Markdown file in the
  repository, code blocks included, at 85 columns (`.prettierrc.json`); CI
  runs `bun run format:check`.
- A new Markdown file anywhere in the repository appears in the sidebar on
  the next build; nothing here lists pages by hand.
