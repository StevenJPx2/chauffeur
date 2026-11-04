---
name: chauffeur-docs
description: MUST use when writing or editing any Markdown in the Chauffeur repository, including the slice pages in docs/plans/slices/, the plan, plugin, integration, CLI, tools and migration pages, READMEs and code examples in them. Covers how a slice page is laid out by its status, how code examples are written and spaced, and how the docs site is checked and published.
---

# Chauffeur docs

Every Markdown file in the repository is a page on the docs site, built by
VitePress in `site/` and published to
`https://chauffeur-docs.stevenjpx2.workers.dev`.
Write each page as the first and only draft of the current design.

## Voice

- Describe Chauffeur as it is, in the present tense, as if it exists. No
  revision history: never "previously", "now", "instead of", "renamed from",
  "no longer", or a correction of an earlier draft.
- Plain language, top down: what it does for a person first, then how, each
  with a concrete example such as `fixed stuff` getting the nudge.
- State a decision once, in its canonical page, and link to it elsewhere.
- No sibling project is presented as something Chauffeur depends on.
  Integrations (sourcefed, neo, jev-distill, opencode-history) are their own
  projects that call Chauffeur.
- Name events by their owner, such as `sourcefed.event` or `neo.screen-check`.
  `tool`, `prompt`, `permission`, `response`, `session` and `compaction`
  belong to harnesses.
- The commands section is titled "CLI". A plugin's commands go under its ID,
  such as `chauffeur skills list`, in a Commands section on its page and in
  [CLI](../../../docs/cli/README.md#commands-from-plugins).
- Something designed but not built is marked by its slice's status, not by
  hedges in the prose.

## Code examples

Show how a thing is used, not how it is defined: a call with real arguments
and its result, a rule-file fragment, a command and what it prints. Show a
definition only when its shape is the design, such as a wire message, and
then its smallest form.

Give code room to breathe. One blank line separates each step of a snippet:
setup, the call, checking the result.

```rust
let step = Step::new("commit", "system-one", "Is this message conventional?");
let plan = Plan::new([step])?;

let yes = |_: &&str| Outcome::Value("yes");
let run = plan.run(&Panel::new().with("system-one", &yes))?;

assert_eq!(run.verdict.get("commit"), Some(&Outcome::Value("yes")));
```

- A braced block (`if`, `for`, `while`, `match`) gets a blank line above
  and below, except at the start or end of its scope.
- A one-line guard such as `if (!e.first) return` gets a blank line after
  it; a run of guards stays together.
- Never two blank lines in a row, and never a blank line at the start or end
  of a scope. A two- or three-line snippet is one step and stays dense.
- Every line fits in 85 columns, prose and code. Prettier keeps prose line
  breaks as written, so wrap prose by hand; only a link that cannot break may
  run past. In code, split a long call over lines, or name a value first
  (`let keys = …;`). A comment that would run past goes on its own line
  above the code, as a sentence.
- `…` stands for what does not matter, such as `{ "text": "…" }`.
- Mermaid diagrams must render: check them in the built site.

## Slice pages

`docs/plans/slices/slice-N.md`, one per slice, listed in the slices
`README.md` with its status. Each page has, in order:

1. `# Slice N: <the feature>`, then `**Status:**` (Implemented, In progress,
   Planned or Held, with what is left or what it waits on).
2. `**You can:**`: what a person can do once it ships, with an example.
3. A Mermaid diagram of the path through the feature, when one helps.
4. `## What it builds`: a `###` heading per part, each with a short
   overview, `Files:`, a code example, `Structure:` when the status calls
   for it, `Commands:` when a plugin adds any, and `Verified by:`.
5. `## End to end`: the checks in the real harness or caller.

How much a part shows depends on the slice's status:

| Status                   | Each part shows                                     |
| ------------------------ | --------------------------------------------------- |
| In progress, Implemented | Overview, files, usage, **Structure**, verification |
| Planned, Held            | Overview, files, usage, verification                |

When a slice moves to In progress, write its `Structure:` from the code; when
the code changes, change it in the same commit.

### Structure

A high-level account of each type in the part, read from the code, not a walk
through its steps. For each type, in one bullet:

- what it is, in a few words;
- its fields, named, and which are public;
- each method or function and what it does for its caller: what it refuses,
  returns or changes;
- the traits it implements by hand, such as `Display`, `Error`, `Default`,
  or a crate's own trait such as `Judge` or `Format`, and what that
  implementation does.

Enums list their variants. Derived traits go in one closing sentence for the
part. Private helpers appear only as what a public method does. Constants
appear as values in the prose, such as "at most 64 connections".

```markdown
- **`Fallback`**, a tier's models in order. Field: `models`.
  - `build(&models, tier, preferred, key)` puts `preferred` first, then the
    tier's order, each once. It leaves out a model whose key is missing.
  - `answer_all(&questions, &evidence)` asks each model only the questions
    still open; a question no model answered fails as `Unavailable`.
```

## Other pages

- **Plugins**: one page per plugin in `docs/plugins/`, each with its hooks as
  code, its Commands and its Options. Integrations are in `docs/integrations/`.
- **Migration**: one page per v1 feature in `docs/migration/`. Rule files
  move by a prompt an agent follows to rewrite them, given in the migration
  guide; there is no `migrate` command.
- **Module READMEs**: each Rust module folder has a `README.md` saying what
  its files hold, with one example of its use.

## Checking and publishing

From `site/`:

```sh
bun run format        # Prettier over every Markdown file in the repository
bun run format:check
bun run build         # fails on a dead link
```

Look at changed pages in `bun run preview` before committing. Commit with a
Conventional Commits message, such as `docs(plan): …` or `docs(plugins): …`,
and push to `v2`. The `docs.yml` workflow checks the format, builds and
deploys; watch it to the end.
