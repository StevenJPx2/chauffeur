# Chauffeur

Chauffeur rides along with your OpenCode agent. It watches what the agent is
about to do and steps in when it matters: it hands the agent the right skill,
approves the permission prompts you would have approved anyway, stops the
shortcuts you would have stopped, and keeps a goal running until it is done.

Each decision is a quick yes/no judgment by [Jev](https://typesafe.ai), a small
fast model, so it adds about a third of a second, not another agent turn.

## What it does for you

### The right skill, without the token bill

Chauffeur reads your message and attaches only the skills it needs. It hides
OpenCode's full skill list and the tool groups a task won't use, and brings them
back if a later message does.

![Chauffeur attaching the jira-cli skill to a question about a Jira ticket](docs/media/jira.gif)

![Artifact tools hidden for a rename, then brought back when the next message asks to publish](docs/media/tools.gif)

The decision feed under OpenCode is `chauffeur audit --follow --brief`.

On our longer benchmark tasks, this used 29% fewer input tokens and finished
19% faster than plain OpenCode, with the same or better pass rate
([`bench/`](bench)).

### Fewer permission prompts, and the right ones

- **Approves** what clearly serves your task: an edit in the project, a file
  you named, a sibling worktree of the same repository when the agent says why.
- **Asks you** about the rest: unrelated folders, credential stores such as
  `~/.ssh`, anything it is unsure of.
- **Blocks** irreversible harm (`rm -rf /`) and shell rewrites of project files
  (`sed -i`, `node -e`, heredocs), pointing the agent at the edit tools, so every
  change stays reviewable. A force-push to `main` always asks.

![Chauffeur denying a sed rewrite; the agent switches to the edit tool](docs/media/guard.gif)

### Steers when the agent drifts

After a tool call, Chauffeur nudges the agent back on course, and hands over the
matching skill when there is one: `cat` or `sed -n` instead of the read tool,
a recursive `grep` instead of `rg`, a browser for GitHub, Slack, Jira, or X
where `gh`, `slackcli`, `jira`, or `twitter-cli` does it directly. A project
can add its own rules in `.chauffeur/rules/`.

![The agent prints two files with cat; Chauffeur steers it to the read tool](docs/media/steer.gif)

### Rulebooks: `/goal`, `/ticket`, and your own

A rulebook is a set of rules you switch on for one session with a slash command.

- **`/goal <objective>`** keeps the agent working across turns until the
  evidence shows the goal is met, pauses when only you can unblock it, and stops
  after 20 continuations with a progress summary.
- **`/ticket <Jira key | Slack link | request>`** takes an HPDP Overlay task
  from intake to a PR in review. It is scoped to those folders, so it only
  appears there.

![/goal running a task to "Goal achieved"](docs/media/goal.gif)

Add your own in `skills/rulebooks/`, or per project in `.chauffeur/rulebooks/`
([format](skills/README.md#rulebooks)).

### Ask for what's missing

With **`ask_chauffeur`**, the agent can ask in plain words for a tool or skill
it lacks, then use what Chauffeur grants to finish the task.

![The agent asks Chauffeur for a Slack-posting skill and uses slackcli to finish the handoff](docs/media/ask.gif)

### And quietly

- **Model failover**: on a usage limit, Chauffeur switches to an equivalent
  model and back once the limit has likely cleared.
- **[sourcefed](https://github.com/StevenJPx2/sourcefed) events**: Chauffeur sets up
  monitors for the PR, Jira issue, or Slack thread you are working on, and lets
  through only the events the agent needs to act on.

## Install

You need Rust, Node.js, OpenCode 2, and a TypeSafe API key.

```sh
export TYPESAFE_API_KEY=...          # Jev; the daemon will not start without it
export CHAUFFEUR_IDLE_STEERING=true  # lets rulebooks continue the agent between turns

git clone https://github.com/StevenJPx2/chauffeur && cd chauffeur
cargo install --locked --path crates/cli             # the chauffeur daemon and CLI
mkdir -p ~/.config/chauffeur && ln -s "$PWD/skills" ~/.config/chauffeur/skills
ln -s "$PWD"/skills/handoff/* ~/.agents/skills/      # skills Chauffeur hands over
cd adapters/opencode && npm install && npm run deploy
opencode service restart
```

`npm run deploy` checks and builds the plugin, then installs it as
`~/.config/opencode/plugins/chauffeur.js`. The plugin starts the daemon when
none is running. If Jev is unreachable, Chauffeur stands aside: prompts go
through unchanged and permission requests fall back to asking you.

## See what it decided

Every decision is logged, with the questions asked and how long they took:

```sh
chauffeur audit                   # the last 20 decisions
chauffeur audit 100
chauffeur audit --follow --brief  # new decisions as they happen, one short line each
```

```text
00:16:40 UTC ses_f32a1b… user_message Check my latest Twitter mentions → attach_skills ["twitter-cli"] | 1 asked, 493 ms
00:16:42 UTC ses_f32a1b… permission_request shell rm -rf / → permission deny | 0 asked, 0 ms; vetoed: rm -rf /
```

## Configure

| Variable | Default |
|---|---|
| `TYPESAFE_API_KEY` | required |
| `CHAUFFEUR_IDLE_STEERING` | `false`; `true` lets turn-end rules and rulebooks resume the agent |
| `CHAUFFEUR_CONFIG_DIR` | `~/.config/chauffeur`; per-capability overrides as JSON |
| `CHAUFFEUR_SKILLS_DIR` | `~/.config/chauffeur/skills` (a link to `skills/`) |
| `CHAUFFEUR_STATE_DIR` | `~/.local/state/chauffeur` (session memory, audit log) |
| `CHAUFFEUR_DAEMON_URL` | `http://127.0.0.1:18790` |
| `CHAUFFEUR_SOURCEFED` | on; `off` leaves sourcefed alone |

Rules, rulebooks, and permission contracts are strict JSON in [`skills/`](skills);
[`skills/README.md`](skills/README.md) documents each format, and
`chauffeur skill validate PATH` checks a file. [`ARCHITECTURE.md`](ARCHITECTURE.md)
covers the design.

Edits apply without a restart. Within a second, the daemon picks up rules,
rulebooks, and contracts in `skills/` and overrides in `~/.config/chauffeur/`,
keeping session memory and running rulebooks. A new or removed rulebook's slash
command appears or disappears in OpenCode within a few seconds. A file that
fails to load leaves the previous version running and logs why.

## Develop

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cd adapters/opencode && npm run check   # types, lint, tests, fallow audit
```

`chauffeur-bench` compares OpenCode with and without Chauffeur on a task suite;
see [`bench/README.md`](bench/README.md).

The demos above were recorded with
[terminal-control](https://github.com/anomalyco/terminal-control) in throwaway
repositories, with stub `jira` and `slackcli` CLIs. The ask demo's `AGENTS.md`
requires a Slack handoff and supplies the question the agent asks Chauffeur.
