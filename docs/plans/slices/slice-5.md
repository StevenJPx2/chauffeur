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

## What it builds

### The plugin host

`plugins.json`; the stdio plugin protocol; the discovery cache, lazy and eager
start, idle stop, restart with backoff and reload on file change. `ctx.on` and
`ctx.before` routing, `before` budgets and `onTimeout`, retries by kind, and
the stale-action check.

Files: `crates/daemon/src/plugins.rs`, `bus.rs`

Verified by:

- A plugin crash, a missed `before` budget and a stale action each behave as
  specified.
- `onTimeout: "ask"` hands a tool call to the user.
- Discovery records the same hooks a full start registers.
- `main`'s `rpc-starvation.test.ts` passes.

### The person tier

The `person` format: a question shown to the user through the harness.

Files: `crates/judges/src/tiers/person/`

Verified by:

- A person step reaches the harness and settles to the user's answer.

### The SDK

`@fdcn/chauffeur/plugin` and `/harness`.

Files: `packages/chauffeur/src/plugin/`, `harness/`

Verified by:

- The first-party plugins of this slice build and run on it.

### The `guard` plugin

The backstop and `confirm` patterns, the irreversible-harm question and its
learned list. A spike decides whether first-party plugins ship compiled with
scriptc: it compiles `guard`, and reports which SDK and Node APIs scriptc
rejects, start time and memory against Bun, and build time. User plugins,
and first-party plugins in development, run from source on Bun.

Files: `plugins/guard/`

Verified by:

- `main`'s `read-guard-probes.json` passes.

### The `sourcefed` plugin

Starts monitors for the session's own PRs, Jira issues and Slack threads;
sourcefed's installer adds it to `plugins.json` with `sourcefed/triage`.

Files, in sourcefed: `adapters/chauffeur/` and its installer

Verified by:

- v1's monitor cases: a PR the agent opened is followed, one it only looked
  up is not, and an unreachable sourcefed creates nothing.

### Rule-file fields for v1's rules

The `when` filters (`input.excludes`, `result`, `agent`, `reason`, a
permission's `actions`, `host` and `resources`, `history`), the evidence
blocks each event gives, `priority`, `once`, and `permission.reply` with the
most restrictive reply winning.

Files: `crates/protocol/src/when.rs`, `then.rs`

Verified by:

- Each filter admits and skips the cases in [Rule files: v1 to
  v2](../../migration/rule-format.md).

### v1's rules, ported

The steering, turn-end and permission rules.

Files: `distribution/rules/`, `distribution/rules/permission/`

Verified by:

- `main`'s rule tests, labelled tool calls and recorded permission requests
  give the same nudges and replies.

### Learned credential shapes

Shapes a judge confirmed as secret, kept in Chauffeur's state.

Files: `crates/runtime/src/redact/learned.rs`

Verified by:

- A learned shape is redacted on the next run, and a reviewed-out one is not.

### Commands

`chauffeur daemon status --project`; plugins' own commands, recorded with
`ctx.command` at discovery, listed in `--help`, and run in the plugin through
the daemon's `chauffeur.command`.

Files: `crates/cli/src/daemon.rs`, `plugin.rs`, `crates/daemon/src/commands.rs`

Verified by:

- A plugin's command runs while the plugin is stopped, and a plugin named
  after a built-in command is refused.

## End to end

- In a real OpenCode, a destructive command is blocked or asked about, and
  `ls` runs within the budget.
