# Slice 5: Harm guard, permissions and v1's rules

**Status:** Planned.

**You can:** have the guard ask System One, before a shell command runs,
whether it could cause irreversible harm. `yes` blocks the call; `unsure`,
`failed` or a slow answer hands it to you. Chauffeur also approves the
permission prompts that serve your task, blocks shell rewrites of project
files, and steers with every v1 rule that needs no skill, as in
[Permissions](../../migration/permission.md),
[Steering after a tool call](../../migration/steering-rules.md) and
[Steering at the end of a turn](../../migration/turn-end-rules.md).

This slice adds [plugins](../pluggability.md#plugins), the person tier, and the rule-file
fields v1's rules need: `when`'s `input.excludes`, `result`, `agent`,
`reason`, permission and `history` filters, the evidence blocks each event
gives, `priority` and `once`.

| File                                                                 | Contents                                                                                                                                                                        |
| -------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/daemon/src/plugins.rs`                                       | `plugins.json`; the stdio plugin protocol; discovery cache, lazy and eager start, idle stop, restart with backoff, reload on file change                                        |
| `crates/daemon/src/bus.rs`                                           | `ctx.on` and `ctx.before` routing, `before` budgets and `onTimeout`, retries by kind, the stale-action check                                                                    |
| `crates/judges/src/tiers/person/`                                    | The `person` format: a question shown to the user through the harness                                                                                                           |
| `packages/chauffeur/src/{plugin,harness}/`                           | `@fdcn/chauffeur/plugin` and `/harness`                                                                                                                                         |
| `plugins/guard/`                                                     | The backstop and `confirm` patterns, the irreversible-harm question and its learned list                                                                                        |
| sourcefed: `adapters/chauffeur/` and its installer                   | The `sourcefed` plugin, which starts monitors for the session's own PRs, Jira issues and Slack threads, added to `plugins.json` with `sourcefed/triage`                         |
| `crates/protocol/src/{when,then}.rs`                                 | The `when` filters, `priority`, `once`, and `permission.reply` with the most restrictive reply winning                                                                          |
| `distribution/rules/`, `distribution/rules/permission/`              | v1's steering, turn-end and permission rules, ported                                                                                                                            |
| `crates/runtime/src/redact/learned.rs`                               | Learned credential shapes, kept in Chauffeur's state                                                                                                                            |
| `crates/cli/src/{daemon,plugin}.rs`, `crates/daemon/src/commands.rs` | `chauffeur daemon status --project`; plugins' own commands: `ctx.command` recorded at discovery, listed in `--help`, run in the plugin through the daemon's `chauffeur.command` |

A spike decides whether first-party plugins ship compiled with scriptc. It
compiles `guard`, and reports which SDK and Node APIs scriptc rejects, start
time and memory against Bun, and build time. User plugins, and first-party
plugins in development, run from source on Bun.

Verification: a plugin crash, a missed `before` budget and a stale action each
behave as specified; `onTimeout: "ask"` hands a tool call to the user;
discovery records the same hooks a full start registers; `main`'s
`read-guard-probes.json` and `rpc-starvation.test.ts` pass; `main`'s rule
tests, labelled tool calls and recorded permission requests give the same
nudges and replies; in a real OpenCode, a destructive command is blocked or
asked about and `ls` runs within the budget.
