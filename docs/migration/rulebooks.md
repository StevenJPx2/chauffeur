# Rulebooks

A v1 rulebook is a set of rules a slash command switches on for one session:
`/goal <objective>` keeps the agent working until the evidence shows the goal
is met, `/ticket` does the same for a ticket, and a project adds its own in
`.chauffeur/rulebooks/`.

| v1                                                           | v2                                                         |
| ------------------------------------------------------------ | ---------------------------------------------------------- |
| `skills/rulebooks/goal.json`, `ticket.json`                  | `distribution/rulebooks/`, run by the `rulebooks` plugin   |
| `args`, `budget` (20 continuations), `on_start`, `on_budget` | The same fields                                            |
| Rules inside the book, in the v1 rule format                 | Rules in the v2 format, as in [Rule files](rule-format.md) |
| `"end": "complete"` or `"pause"` in a rule's `then`          | An `end` intent the plugin carries out                     |
| `when.todos`: `none`, `done`                                 | The `todos` evidence block, and `when.todos`               |
| An optional System Two completion review                     | A System Two step with `after`, from slice 11              |

The [`rulebooks` plugin](../plugins/rulebooks.md) runs them. The design is
held until it is discussed: see the plan's
[Rulebooks](../plans/pluggability.md#rulebooks) and
[Slice 12](../plans/slices/slice-12.md). Cutover waits for it,
since v1 users rely on `/goal`.
