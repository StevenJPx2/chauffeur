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

Commands: `chauffeur skills list`, `show` and `approve`.

```ts
ctx.before(
  "prompt.sent",
  async (e) => {
    const candidates = await closest(
      await skill.list(e.session),
      e.text,
      ctx.options.ranking
    )
    // A yes/no per skill, then select keeps the best four.
    const best = await pick.run({ evidence: { message: e.text }, candidates })
    if (best.ok) e.append(best.outcome.map((id) => ({ skill: id })))
  },
  { budget: 400, onTimeout: "allow" }
)
```

Verified by:

- `main`'s `sources/tests.rs` vetting cases.
- A skill from an HTTP catalog in OpenCode's `skills` config is judged and
  attached like a local one.

### The `tools` plugin

Hiding and revealing tool groups, Code Mode notes, shorter built-in
descriptions, and missing-tool recovery.

Files: `plugins/tools/`

Commands: `chauffeur tools list` and `reveal`.

```ts
ctx.before(
  "prompt.sent",
  async (e) => {
    if (!e.first) return
    // A yes/no per group outside base; hidden when no reaches 0.7.
    const hidden = await unneededGroups(e)
    session.update(e.session, { tools: { hidden } })
  },
  { budget: 400, onTimeout: "allow" }
)
```

Verified by:

- `main`'s `tool-exposure/tests/exposure.rs`.

### The `ask-chauffeur` plugin

The `ask_chauffeur` tool, through the tool registry.

Files: `plugins/ask-chauffeur/`

Commands: `chauffeur ask-chauffeur try`.

```ts
tool.add({
  name: "ask_chauffeur",
  description: "Ask for a tool or skill you lack, in plain words.",
  input: { need: "string" },
  run: (input, e) => grant(input.need, e)
})
```

Verified by:

- `main`'s `bench/ask` requests hand over the same tools and skills.

### Hand-over rules and skills

The steers that hand over a skill, and the skills they hand over, added to
the harness's skill sources when Chauffeur is installed.

Files: `distribution/rules/jira-via-browser.json`, `slack-via-browser.json`,
`twitter-via-browser.json`, `distribution/skills/handoff/`

```json
{
  "message.append": {
    "parts": [
      { "text": "Chauffeur: Use slackcli for Slack instead of the browser; …" },
      { "skill": "slack-cli" }
    ],
    "delivery": "now"
  }
}
```

Verified by:

- `main`'s labelled calls give the same nudges, each with its skill.

## End to end

- In a real OpenCode, a request to review a pull request attaches the review
  skill.
