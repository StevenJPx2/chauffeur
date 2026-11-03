# Moving from v1

Every v1 feature ships in v2, rebuilt on v2's pieces: rule files for
decisions that are data, plugins for behaviour that keeps state or acts over
time, and the harness package for the harness. Each page shows v1's file and
the v2 file or plugin that replaces it; each plugin has its own page under
[Plugins](../plugins/README.md). Cutover, slice 13, waits until every
row has landed and passes v1's cases.

| v1 feature                                           | v2                                                                            | Slice                  | Page                                                  |
| ---------------------------------------------------- | ----------------------------------------------------------------------------- | ---------------------- | ----------------------------------------------------- |
| Rule format, evidence                                | Rule files                                                                    | 1, 5                   | [Rule files: v1 to v2](rule-format.md)                |
| Steering after a tool call (11 rules)                | Rules on `tool.after`                                                         | 1, 5, 6                | [Steering after a tool call](steering-rules.md)       |
| Steering at turn end (4 rules)                       | Rules on `response.finished`                                                  | 5                      | [Steering at the end of a turn](turn-end-rules.md)    |
| Permission contracts, shell guards, backstop         | Rules on `permission.asked`, the `guard` plugin                               | 5                      | [Permissions](permission.md)                          |
| Redaction                                            | The engine                                                                    | 1, learned shapes in 5 | [Redaction](redaction.md)                             |
| Judge choice, fallback, routes                       | `models.json`, a step's `judge`                                               | 1, 3, 4, 11            | [Judges](judges.md)                                   |
| sourcefed event gate and monitors                    | `sourcefed/triage` and the `sourcefed` plugin, both in sourcefed's repository | 2, 5                   | [sourcefed events and monitors](sourcefed.md)         |
| Skill exposure, drift, remote sources, review        | The `skills` plugin                                                           | 6                      | [Skills](skills.md)                                   |
| Tool exposure, Code Mode notes, shorter descriptions | The `tools` plugin                                                            | 6                      | [Tools](tools.md)                                     |
| `ask_chauffeur`                                      | The `ask-chauffeur` plugin                                                    | 6                      | [`ask_chauffeur`](ask-chauffeur.md)                   |
| Model failover and switch-back                       | The `model-router` plugin                                                     | 7                      | [Model router](model-router.md)                       |
| Context renewal, session warming                     | The `context-renewal` and `warming` plugins                                   | 7                      | [Context renewal and warming](renewal-and-warming.md) |
| `/goal`, `/ticket`, project rulebooks                | The `rulebooks` plugin                                                        | 12                     | [Rulebooks](rulebooks.md)                             |
| Audit feed, rule validation                          | `chauffeur audit`, `chauffeur daemon status`                                  | 1, 5                   | [Seeing what it decided](audit.md)                    |

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

## Rewriting your configuration

v2 reads no v1 file, so nothing is converted behind your back. To move your
own rules and settings, give an agent this prompt:

```text
Rewrite my Chauffeur v1 configuration as Chauffeur v2 files.

First read https://chauffeur-docs.stevenjpx2.workers.dev/docs/migration/ and
every page it links, especially "Rule files: v1 to v2".

1. Copy ~/.config/chauffeur/ to ~/.config/chauffeur.v1/ and work from the copy.
   Change nothing in the copy.
2. Find the v1 files: the copy's rules/, permission/, backstop.json,
   redaction.json and config files, every .chauffeur/rules/ and
   .chauffeur/rulebooks/ in my projects under ~/Documents/Projects, and the
   CHAUFFEUR_* variables in my shell profile and Homebrew service file.
3. Skip what v2 already ships: the rules in Chauffeur's own skills/ folder are
   ported. Rewrite only mine, and settings I changed from v1's defaults.
4. Write each v1 rule as a v2 rule file in the matching v2 folder under
   ~/.config/chauffeur/ or the project's .chauffeur/, keeping
   every instruction and message word for word. Put settings where "Where
   configuration goes" says: models.json, plugins.json options, settings.json.
5. Run `chauffeur daemon status` and fix every refused rule until it lists
   none.
6. Do not guess. For anything with no v2 home, leave it out and list it.

Finish with a table: each v1 file, the v2 file it became, and what was left
out and why.
```
