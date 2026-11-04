---
name: chauffeur-docs
description: MUST use when writing or editing any Markdown in the Chauffeur repository, including the slice pages in docs/plans/slices/, the plan, plugin, integration, CLI, tools and migration pages, and module READMEs. Adds Chauffeur's names, page layout and docs site checks to the spec-docs skill.
---

# Chauffeur docs

Follow the `spec-docs` skill: its voice, code examples, line length, slice
pages and Structure apply here. This adds what is Chauffeur's own.

## The site

Every Markdown file in the repository is a page on the docs site, built by
VitePress in `site/` and published to
`https://chauffeur-docs.stevenjpx2.workers.dev`. `.agents/` and build
output are left out.

```sh
cd site
bun run format         # Prettier over every Markdown file, 85 columns
bun run format:check
bun run build          # fails on a dead link
bun run preview        # look at the changed pages
```

Push to `v2`; the `docs.yml` workflow checks the format, builds and deploys.
Watch it to the end.

## Names

- Describe Chauffeur as if it exists. No sibling project is presented as
  something it depends on: the integrations (sourcefed, neo, jev-distill,
  opencode-history) are their own projects that call Chauffeur.
- Events are named by their owner, such as `sourcefed.event` or
  `neo.screen-check`. `tool`, `prompt`, `permission`, `response`,
  `session` and `compaction` belong to harnesses.
- The commands section is titled "CLI". A plugin's commands go under its ID,
  such as `chauffeur skills list`, in a Commands section on its page and in
  `docs/cli/README.md`'s Commands from plugins.

## Pages

- **Slices**: `docs/plans/slices/slice-N.md`, listed in its `README.md`.
  Each part has `Files:`, a code example, `Structure:` when the slice is
  In progress or Implemented, `Commands:` when a plugin adds any, and
  `Verified by:`.
- **Plugins**: one page per plugin in `docs/plugins/`, with its hooks as
  code, its Commands and its Options. Integrations are in
  `docs/integrations/`.
- **Migration**: one page per v1 feature in `docs/migration/`. Rule files
  move by a prompt an agent follows to rewrite them, given in the migration
  guide; there is no `migrate` command.
- **Module READMEs**: each Rust module folder has a `README.md` saying what
  its files hold, with one example of its use.
- A rule's `instructions` and `text` are long JSON strings; they may run
  past 85 columns.
