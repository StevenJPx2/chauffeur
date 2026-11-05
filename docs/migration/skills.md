# Skills

v1's skill exposure attached the skills a message needs, hid OpenCode's own
skill list, handed over a better skill when the agent drifted, served skills
from remote sources, and reviewed third-party skills before agents followed
them. In v2 it is the `skills` plugin, in slice 6, over the skills the
harness already knows: its own folders and configured sources, which the
plugin reads through its adapter's `skill` registry. Its hooks,
options and sources are on the [`skills` plugin's page](../plugins/skills.md).

| v1                                                                                                                              | v2                                                                                                                                                                        |
| ------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Hiding the `skill` tool and its list with an `agent.transform` deny rule                                                        | The plugin removes the `skill` tool with the harness's tool registry at start; skills it attaches still resolve, and `@skill-id` still loads one                          |
| At each message, one yes/no per candidate skill, attaching those at P ≥ 0.7                                                     | `ctx.before("prompt.sent")`: `for_each` over the candidates, then `select` with `top: 4`, `min: 0.7`, attached as `skill` parts                                           |
| Candidates: BM25 over ID and description, the closest 16, plus `ranking.always`, skills the message names and the project's own | A `bm25` fact step picks the candidates; the plugin adds the pinned, named and project skills                                                                             |
| `CHAUFFEUR_RANKING=embeddings`                                                                                                  | The `embedding` fact in place of `bm25`, chosen in the plugin's options                                                                                                   |
| Three wordings: a project's own skill (attach unless a confident no), a skill the message names, any other                      | Three `asks` in the plugin's strategy, picked per candidate by the same facts                                                                                             |
| At most four skills and 64 KiB per message                                                                                      | The same bounds, in the plugin                                                                                                                                            |
| Drift: after a tool result (once a minute) and at turn end, one `choice` of a skill that does the work better                   | `ctx.on("tool.after")` and `ctx.on("response.finished")`: the same `choice`, delivered `now` mid-turn and `next` at turn end                                              |
| Remote sources: Git repositories and HTTP catalogs, up to 32, fetched by the daemon                                             | The harness's sources. In OpenCode, `skills` in `opencode.json` lists local folders and HTTP catalogs, and OpenCode downloads and caches them; the plugin fetches nothing |
| Skill review: System One screens, System Two reviews what is unsure                                                             | A screen-then-review strategy: System One in slice 6, System Two from slice 11                                                                                            |
| Handover skills in `skills/handoff/`, linked into a folder OpenCode reads                                                       | `distribution/skills/handoff/`, added to the harness's skill sources when Chauffeur is installed                                                                          |

`skills/config/skill-exposure.json` becomes the plugin's
[options](../plugins/skills.md#options).
