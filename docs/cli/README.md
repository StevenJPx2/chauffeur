# The `chauffeur` command

One binary runs the daemon and talks to it. Its name is the installation's:
`chauffeur`, or `chauffeur-next` for a release candidate, which keeps its own
socket, configuration and audit log beside an installed `chauffeur`.

| Command                        | Does                                                             | Slice |
| ------------------------------ | ---------------------------------------------------------------- | ----- |
| [`daemon`](daemon.md)          | Runs the daemon in the foreground                                | 1     |
| [`emit`](emit.md)              | Sends one harness event and prints what the rules returned       | 1     |
| [`audit`](audit.md)            | Prints the runs in the audit log, and follows it                 | 1     |
| [`status`](status.md)          | Prints the daemon's version, loaded rules and refused rule files | 1     |
| [`version`](version.md)        | Prints the installation's name and version                       | 1     |
| [`navigate`](navigate.md)      | Sends one request as a caller does and prints the messages back  | 2     |
| [`strategy show`](strategy.md) | Draws a rule's strategy as a tree, with its bounds               | 3     |
| [`skills`](skills.md)          | Lists the skills the skills plugin sees, and their screens       | 6     |
| [`migrate`](migrate.md)        | Writes v2's files from a v1 configuration                        | 13    |

## Where things are

| What               | Where, for the name `chauffeur`                                                   | Set with                                             |
| ------------------ | --------------------------------------------------------------------------------- | ---------------------------------------------------- |
| The socket         | `$XDG_RUNTIME_DIR/chauffeur/daemon.sock`, or under `$TMPDIR`                      | `CHAUFFEUR_SOCKET`                                   |
| Your configuration | `~/.config/chauffeur/`: `models.json`, `rules/`, `redaction.json`, `plugins.json` | `XDG_CONFIG_HOME`                                    |
| Shipped content    | `share/chauffeur/` beside the binary's folder                                     | `CHAUFFEUR_DISTRIBUTION`, or `daemon --distribution` |
| The audit log      | `~/.local/state/chauffeur/audit.jsonl`                                            | `XDG_STATE_HOME`                                     |
| Model keys         | The environment the daemon runs in, such as `TYPESAFE_API_KEY` for Jev            | Each model's `key` in `models.json`                  |

Every command exits 0 when it did what it was asked, and 1 with a message on
stderr when it could not.
