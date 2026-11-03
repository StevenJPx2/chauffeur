# Moving from v1

Every v1 feature ships in v2, rebuilt on v2's pieces: rule files for
decisions that are data, plugins for behaviour that keeps state or acts over
time, and the harness package for the harness. Each page shows v1's file and
the v2 file or plugin that replaces it; each plugin has its own page under
[Plugins](../plugins/README.md). Cutover, slice 13, waits until every
row has landed and passes v1's cases.

| v1 feature                                           | v2                                              | Slice                  | Page                                                  |
| ---------------------------------------------------- | ----------------------------------------------- | ---------------------- | ----------------------------------------------------- |
| Rule format, evidence                                | Rule files                                      | 1, 5                   | [Rule files: v1 to v2](rule-format.md)                |
| Steering after a tool call (11 rules)                | Rules on `tool.after`                           | 1, 5, 6                | [Steering after a tool call](steering-rules.md)       |
| Steering at turn end (4 rules)                       | Rules on `response.finished`                    | 5                      | [Steering at the end of a turn](turn-end-rules.md)    |
| Permission contracts, shell guards, backstop         | Rules on `permission.asked`, the `guard` plugin | 5                      | [Permissions](permission.md)                          |
| Redaction                                            | The engine                                      | 1, learned shapes in 5 | [Redaction](redaction.md)                             |
| Judge choice, fallback, routes                       | `models.json`, a step's `judge`                 | 1, 3, 4, 11            | [Judges](judges.md)                                   |
| sourcefed event gate and monitors                    | `sourcefed/triage`, the `monitors` plugin       | 2, 5                   | [sourcefed events and monitors](sourcefed.md)         |
| Skill exposure, drift, remote sources, review        | The `skills` plugin                             | 6                      | [Skills](skills.md)                                   |
| Tool exposure, Code Mode notes, shorter descriptions | The `tools` plugin                              | 6                      | [Tools](tools.md)                                     |
| `ask_chauffeur`                                      | The `ask-chauffeur` plugin                      | 6                      | [`ask_chauffeur`](ask-chauffeur.md)                   |
| Model failover and switch-back                       | The `model-router` plugin                       | 7                      | [Model router](model-router.md)                       |
| Context renewal, session warming                     | The `context-renewal` and `warming` plugins     | 7                      | [Context renewal and warming](renewal-and-warming.md) |
| `/goal`, `/ticket`, project rulebooks                | The `rulebooks` plugin                          | 12                     | [Rulebooks](rulebooks.md)                             |
| Audit feed, rule validation                          | `chauffeur events`, `chauffeur status`          | 1, 5                   | [Seeing what it decided](audit.md)                    |

## Where configuration goes

| v1                                                  | v2                                                                                            |
| --------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| `skills/rules/*.json`                               | `distribution/rules/*.json`                                                                   |
| `skills/permission/*.json`                          | `distribution/rules/permission/*.json`                                                        |
| `skills/safety/*.json`                              | `distribution/safety/*.json`                                                                  |
| `skills/config/<capability>.json`                   | The options of the plugin that replaces the capability, in `~/.config/chauffeur/plugins.json` |
| `skills/config/providers/*.json`                    | `distribution/providers/*.json`                                                               |
| `skills/handoff/*`                                  | `distribution/skills/handoff/*`                                                               |
| `skills/rulebooks/*.json`                           | `distribution/rulebooks/*.json`                                                               |
| `$CHAUFFEUR_CONFIG_DIR/rules/`, `.chauffeur/rules/` | `~/.config/chauffeur/rules/`, `.chauffeur/rules/`                                             |
| `CHAUFFEUR_JUDGE`, `CHAUFFEUR_JUDGE_FALLBACK`       | `~/.config/chauffeur/models.json`                                                             |
| `CHAUFFEUR_IDLE_STEERING=true`                      | `"wake": true` in `~/.config/chauffeur/settings.json`                                         |

At cutover, `chauffeur migrate` writes the v2 files from a v1 configuration
and lists anything it could not carry over.
