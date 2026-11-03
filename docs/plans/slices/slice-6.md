# Slice 6: Skills and tools

**Status:** Planned.

**You can:** have the skills plugin attach the skills each prompt needs, find
skills from every source the harness is configured with, local or remote, and
screen new third-party skills before they are offered.
The tools plugin hides the tools a task will not use, and the agent asks for
what it lacks with `ask_chauffeur`, as in [Skills](../../migration/skills.md),
[Tools](../../migration/tools.md) and [`ask_chauffeur`](../../migration/ask-chauffeur.md).
[Tools](../../tools/README.md) shows how the agent calls it.

## What it builds

### The `skills` plugin

Exposure, drift hand-over and the screen-then-review strategy over the
harness's skill catalog, using `prompt.sent` edits and the `skill` registry.
The review step runs on System Two from slice 11; until then a skill the
screen leaves `unsure` is withheld.

Files: `plugins/skills/`

Verified by:

- `main`'s `sources/tests.rs` vetting cases.
- A skill from an HTTP catalog in OpenCode's `skills` config is judged and
  attached like a local one.

### The `tools` plugin

Hiding and revealing tool groups, Code Mode notes, shorter built-in
descriptions, and missing-tool recovery.

Files: `plugins/tools/`

Verified by:

- `main`'s `tool-exposure/tests/exposure.rs`.

### The `ask-chauffeur` plugin

The `ask_chauffeur` tool, through the tool registry.

Files: `plugins/ask-chauffeur/`

Verified by:

- `main`'s `bench/ask` requests hand over the same tools and skills.

### Hand-over rules and skills

The steers that hand over a skill, and the skills they hand over, added to
the harness's skill sources when Chauffeur is installed.

Files: `distribution/rules/jira-via-browser.json`, `slack-via-browser.json`,
`twitter-via-browser.json`, `distribution/skills/handoff/`

Verified by:

- `main`'s labelled calls give the same nudges, each with its skill.

## End to end

- In a real OpenCode, a request to review a pull request attaches the review
  skill.
