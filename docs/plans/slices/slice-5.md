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

At discovery the daemon sends `start`, and the plugin answers with its hooks:

```json
{ "jsonrpc": "2.0", "id": 1, "method": "start", "params": { "mode": "discovery" } }
```

```json
{
  "jsonrpc": "2.0",
  "method": "hooks",
  "params": {
    "on": ["response.finished"],
    "before": [{ "event": "tool.before", "budget": 2000, "onTimeout": "ask" }]
  }
}
```

Verified by:

- A plugin crash, a missed `before` budget and a stale action each behave as
  specified.
- `onTimeout: "ask"` hands a tool call to the user.
- Discovery records the same hooks a full start registers.
- `main`'s `rpc-starvation.test.ts` passes.

### The person tier

The `person` format: a question shown to the user through the harness.

Files: `crates/judges/src/tiers/person/`

```json
"judges": { "you": { "tier": "person" } },
"strategy": [{ "id": "confirm", "judge": "you", "ask": "force-push" }]
```

Verified by:

- A person step reaches the harness and settles to the user's answer.

### The SDK

`@fdcn/chauffeur/plugin` and `/harness`.

Files: `packages/chauffeur/src/plugin/`, `harness/`

```ts
import { Plugin } from "@fdcn/chauffeur/plugin"
import { message, session, tool } from "@fdcn/chauffeur/harness"

export default Plugin.define({
  id: "notes",
  setup(ctx) {
    ctx.on("response.finished", (e) =>
      message.append(e.session, [{ text: "…" }], { delivery: "next" })
    )
  }
})
```

Verified by:

- The first-party plugins of this slice build and run on it.

### The `guard` plugin

The backstop and `confirm` patterns, the irreversible-harm question and its
learned list. A spike decides whether first-party plugins ship compiled with
scriptc: it compiles `guard`, and reports which SDK and Node APIs scriptc
rejects, start time and memory against Bun, and build time. User plugins,
and first-party plugins in development, run from source on Bun.

Files: `plugins/guard/`

```json
{
  "patterns": ["rm -rf /", "mkfs", "of=/dev/"],
  "confirm": ["push --force origin main"],
  "replace": false
}
```

Verified by:

- `main`'s `read-guard-probes.json` passes.

### The `sourcefed` plugin

Starts monitors for the session's own PRs, Jira issues and Slack threads;
sourcefed's installer adds it to `plugins.json` with `sourcefed/triage`.

Files, in sourcefed: `adapters/chauffeur/` and its installer

```ts
ctx.on("tool.after", async (e) => {
  for (const work of candidates(e)) {
    // a PR from gh pr create, a Jira key, a Slack thread
    const own = await ask(
      noul("Is this the session's own work, which it should keep following?"),
      { judge: quick, evidence: { call: e.input, work } }
    )
    if (own.ok && own.outcome === "yes") await sourcefed.follow(e.session, work)
  }
})
```

Verified by:

- v1's monitor cases: a PR the agent opened is followed, one it only looked
  up is not, and an unreachable sourcefed creates nothing.

### Rule-file fields for v1's rules

The `when` filters (`input.excludes`, `result`, `agent`, `reason`, a
permission's `actions`, `host` and `resources`, `history`), the evidence
blocks each event gives, `priority`, `once`, and `permission.reply` with the
most restrictive reply winning.

Files: `crates/protocol/src/when.rs`, `then.rs`

```json
{
  "when": {
    "tools": ["shell", "execute"],
    "input": { "excludes": ["gh ", "git "] },
    "agent": "top-level",
    "history": {
      "reach": "session",
      "events": ["github:merged"],
      "not_called": ["jira_transition_issue"]
    }
  },
  "priority": 20,
  "once": "session"
}
```

Verified by:

- Each filter admits and skips the cases in [Rule files: v1 to
  v2](../../migration/rule-format.md).

### v1's rules, ported

The steering, turn-end and permission rules.

Files: `distribution/rules/`, `distribution/rules/permission/`

```text
distribution/rules/
├── conventional-commits.json  cat-to-read.json  grep-recursive.json  …
├── ask-with-question-tool.json  git-conflict-loop.json  …
└── permission/
    ├── edit.json  shell.json  outside-directory.json
    └── shell-reads.json  shell-writes.json
```

Verified by:

- `main`'s rule tests, labelled tool calls and recorded permission requests
  give the same nudges and replies.

### Learned credential shapes

Shapes a judge confirmed as secret, kept in Chauffeur's state.

Files: `crates/runtime/src/redact/learned.rs`

In `~/.local/state/chauffeur/learned-redaction.json`:

```json
{
  "secret": ["40 characters of mixed letters and digits, after \"token=\""],
  "safe": []
}
```

Verified by:

- A learned shape is redacted on the next run, and a reviewed-out one is not.

### Commands

`chauffeur daemon status --project`; plugins' own commands, recorded with
`ctx.command` at discovery, listed in `--help`, and run in the plugin through
the daemon's `chauffeur.command`.

Files: `crates/cli/src/daemon.rs`, `plugin.rs`, `crates/daemon/src/commands.rs`

```ts
ctx.command(
  "list",
  { description: "List the skills I see and their screens" },
  async () => listSkills()
)
```

```sh
chauffeur skills list
chauffeur daemon status --project ~/code/acme-web
```

Verified by:

- A plugin's command runs while the plugin is stopped, and a plugin named
  after a built-in command is refused.

## End to end

- In a real OpenCode, a destructive command is blocked or asked about, and
  `ls` runs within the budget.
