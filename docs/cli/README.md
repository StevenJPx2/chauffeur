# CLI

One binary runs the daemon and talks to it. Its name is the installation's:
`chauffeur`, or `chauffeur-next` for a release candidate, which keeps its own
socket, configuration and audit log beside an installed `chauffeur`.

| Command                                         | Does                                                                      | Slice |
| ----------------------------------------------- | ------------------------------------------------------------------------- | ----- |
| [`daemon run`, `daemon status`](daemon.md)      | Runs the daemon; reports its version, loaded rules and refused rule files | 1     |
| [`emit`](emit.md)                               | Sends one harness event and prints what the rules returned                | 1     |
| [`audit`](audit.md)                             | Prints the runs in the audit log, and follows it                          | 1     |
| [`version`](version.md)                         | Prints the installation's name and version                                | 1     |
| [`navigate`](navigate.md)                       | Sends one request as a caller does and prints the messages back           | 2     |
| [`strategy list`, `strategy show`](strategy.md) | Lists the loaded rules' strategies; draws one as a tree                   | 3     |

## Commands from plugins

A plugin adds its own commands under its ID, so they come and go with the
plugin; [Writing a plugin](../plugins/writing.md#commands) shows how.
`chauffeur --help` lists them while the daemon runs, and each needs the
daemon, which starts the plugin if it is stopped.

| Command                                      | Does                                                                         | Plugin                                                      |
| -------------------------------------------- | ---------------------------------------------------------------------------- | ----------------------------------------------------------- |
| `chauffeur guard check`, `learned`, `forget` | Tries a command against the guard; lists and prunes what it learned to block | [`guard`](../plugins/guard.md#commands)                     |
| `chauffeur skills list`, `show`, `approve`   | The harness's skills, their sources and screens; offers a withheld one       | [`skills`](../plugins/skills.md#commands)                   |
| `chauffeur tools list`, `reveal`             | A session's tool groups, shown or hidden; reveals one                        | [`tools`](../plugins/tools.md#commands)                     |
| `chauffeur ask-chauffeur try`                | What `ask_chauffeur` would grant for a need, without granting it             | [`ask-chauffeur`](../plugins/ask-chauffeur.md#commands)     |
| `chauffeur context-renewal status`           | Each provider's summary model, and providers passed over                     | [`context-renewal`](../plugins/context-renewal.md#commands) |
| `chauffeur warming status`                   | Sessions being warmed, and keep-alives left                                  | [`warming`](../plugins/warming.md#commands)                 |
| `chauffeur model-router status`, `forget`    | Sessions moved off a model, providers at a limit; clears one                 | [`model-router`](../plugins/model-router.md#commands)       |
| `chauffeur rulebooks list`                   | The books, and the sessions running them                                     | [`rulebooks`](../plugins/rulebooks.md#commands)             |
| `chauffeur sourcefed monitors`               | What sourcefed follows for a session                                         | [sourcefed](../integrations/sourcefed.md#the-plugin)        |

## Where things are

| What               | Where, for the name `chauffeur`                                                   | Set with                                                 |
| ------------------ | --------------------------------------------------------------------------------- | -------------------------------------------------------- |
| The socket         | `$XDG_RUNTIME_DIR/chauffeur/daemon.sock`, or under `$TMPDIR`                      | `CHAUFFEUR_SOCKET`                                       |
| Your configuration | `~/.config/chauffeur/`: `models.json`, `rules/`, `redaction.json`, `plugins.json` | `XDG_CONFIG_HOME`                                        |
| Shipped content    | `share/chauffeur/` beside the binary's folder                                     | `CHAUFFEUR_DISTRIBUTION`, or `daemon run --distribution` |
| The audit log      | `~/.local/state/chauffeur/audit.jsonl`                                            | `XDG_STATE_HOME`                                         |
| Model keys         | The environment the daemon runs in, such as `TYPESAFE_API_KEY` for Jev            | Each model's `key` in `models.json`                      |

Every command exits 0 when it did what it was asked, and 1 with a message on
stderr when it could not.
