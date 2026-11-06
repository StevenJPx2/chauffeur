---
name: chauffeur-docs
description: MUST use when writing or editing any Markdown in the Chauffeur repository, including the slice pages in docs/plans/slices/, the plan, plugin, integration, CLI, tools and migration pages, and module READMEs, and when a slice is complete and its end-to-end recording is due. Adds Chauffeur's names, page layout, docs site checks and slice recordings to the spec-docs skill.
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
bun run preview        # look at the changed pages and their diagrams
```

The site renders Mermaid, so a diagram that does not parse shows as an error
on its page.

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
  The page opens with its status, what you can do, and the diagram; a
  complete slice's Recording follows, a few sentences on what the video shows
  and then the video, so a reader sees the feature work before any detail.
  What it builds and End to end come after. Each part has `Files:`, a code
  example, `Structure:` when the slice is In progress or Implemented,
  `Commands:` when a plugin adds any, and `Verified by:`.
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

## Recordings

Once a slice is complete and its rc is installed, record the feature working
end to end and embed the video in the slice page's Recording section, right
after the diagram, below a few sentences that say what it shows. The
recording is the reader's proof that the released rc works, so it uses the
real path: the installed `chauffeur-next`, its daemon with the real models, and
the slice's real harness or caller, never a stand-in model. Record again when
what the slice shows changes.

Drive the terminal with `termctrl` and follow the `terminal-control` skill.
[`recording.md`](recording.md) has slice 1's recording as commands to adapt.

- **Two panes, one story.** Record the harness and
  `chauffeur-next audit 0 --follow --brief` as two sessions, both 120 columns
  wide so an audit line fits on one row, then stack them with the harness on top.
  Mark both sessions at the same moments and export each with the same clips,
  so the audit line appears just as the harness reacts.
- **Only the rc acts.** Work in a scratch git repo in the temp folder whose
  `opencode.jsonc` lists `"-chauffeur"`, `"-opencode-history"` and the rc's
  adapter, so v1's plugin stays out of the session and no run remembers the
  one before it. A run where the agent gives the rule nothing to do shows
  nothing; record again until a take shows the feature.
- **Nothing private on screen.** Hide OpenCode's tabs, which show the titles
  of the user's other sessions, with
  `OPENCODE_CLI_CONFIG_CONTENT='{"tabs":{"mode":"off"}}'`, and use
  `opencode --standalone`. Look at frames from the finished video for keys,
  tokens and other projects before committing it.
- **Captions tell the path.** The harness clips carry captions that name each
  step, such as what the agent did and what Jev answered. The audit clips
  carry its command as the caption, which labels the pane.
- **Export the audit pane with `--include-startup`.** Without it, `termctrl`
  drops the empty frames before the first audit line, and the line shows from
  the first frame.
- **Save it as `docs/media/slice-N.mp4`.** H.264 at 1600 pixels wide, under
  1 MB, and embedded with a `<video>` tag, which the site bundles. Keep the
  tag on one line: Markdown reads an HTML block only when its first line holds
  the whole opening tag.

  ```text
  <video src="../../media/slice-1.mp4" controls muted playsinline width="100%"></video>
  ```

- **Leave nothing behind.** Stop the `termctrl` sessions and any daemon you
  started, delete the OpenCode sessions in the scratch directory, and keep the
  `.termctrl` files out of the repository: they hold every byte typed and
  shown.
