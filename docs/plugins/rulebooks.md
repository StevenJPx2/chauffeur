# `rulebooks`

Runs a book of rules the user switches on for one session with a slash
command, such as `/goal ship the parser fix`. Held until its design is
discussed; the sketch is in the plan's
[Rulebooks](../plans/pluggability.md#rulebooks). Slice 12.

| Hook                    | Does                                                                                                      |
| ----------------------- | --------------------------------------------------------------------------------------------------------- |
| Start                   | Registers each book's command, such as `/goal`, with `command.add`                                        |
| The command             | Starts, pauses, resumes or clears the book in that session                                                |
| A running book's events | Runs its rules through `navigate` and carries out their `then`, counting each delivery against the budget |

Books load from `distribution/rulebooks/`, `~/.config/chauffeur/rulebooks/` and
`.chauffeur/rulebooks/`, as rules do. Each session's running books, their
arguments and remaining budget are kept in the plugin's storage.

## Commands

```sh
# The books, and the sessions running each with their budget left.
chauffeur rulebooks list
```
