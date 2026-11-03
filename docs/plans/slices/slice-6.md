# Slice 6: Skills and tools

**Status:** Planned.

**You can:** have the skills plugin attach the skills each prompt needs, find
skills from every source the harness is configured with, local or remote, and
screen new third-party skills before they are offered.
The tools plugin hides the tools a task will not use, and the agent asks for
what it lacks with `ask_chauffeur`, as in [Skills](../../migration/skills.md),
[Tools](../../migration/tools.md) and [`ask_chauffeur`](../../migration/ask-chauffeur.md).
[Tools](../../tools/README.md) shows how the agent calls it.

| File                                                                                       | Contents                                                                                                                                           |
| ------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| `plugins/skills/`                                                                          | Exposure, drift hand-over and the screen-then-review strategy over the harness's skill catalog, using `prompt.sent` edits and the `skill` registry |
| `plugins/tools/`                                                                           | Hiding and revealing tool groups, Code Mode notes, shorter built-in descriptions, missing-tool recovery                                            |
| `plugins/ask-chauffeur/`                                                                   | The `ask_chauffeur` tool, through the tool registry                                                                                                |
| `distribution/rules/{jira,slack,twitter}-via-browser.json`, `distribution/skills/handoff/` | The steers that hand over a skill, and the skills they hand over, added to the harness's skill sources when Chauffeur is installed                 |

The review step runs on System Two from slice 11. Until then, a skill the
screen leaves `unsure` is withheld.

Verification: `main`'s `sources/tests.rs` vetting cases pass; a skill from an
HTTP catalog in OpenCode's `skills` config is judged and attached like a
local one; in a real OpenCode, a request to review a pull
request attaches the review skill.
