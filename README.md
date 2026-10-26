# Chauffeur

Chauffeur rides along with your OpenCode agent. It watches what the agent is
about to do and steps in when it matters:

- It hands the agent the right skill.
- It approves the permission prompts you would have approved anyway.
- It stops the shortcuts you would have stopped.
- It keeps a goal running until it is done.

Each decision is a quick yes/no judgment. The default judge,
[Jev](https://typesafe.ai), is a small fast model, so it adds about a third of
a second, not another agent turn.

## What it does for you

### The right skill, without the token bill

Chauffeur reads your message and attaches only the skills it needs. It also
keeps the rest of the prompt lean:

- It hides OpenCode's full skill list.
- It hides the directly sent tools a task won't use, and brings them back if a
  later message needs them.
- It replaces the longest built-in tool descriptions with shorter ones.
- With Code Mode on, plugin and MCP tools stay in OpenCode's catalog, which
  lists a fixed number of tools; Chauffeur points the agent to the ones a
  request needs.

![Chauffeur attaching the jira-cli skill to a question about a Jira ticket](docs/media/jira.gif)

![Artifact tools hidden for a rename, then brought back when the next message asks to publish](docs/media/tools.gif)

The decision feed under OpenCode is `chauffeur audit --follow --brief`.

Skills can also come from [configured Git repositories and HTTP catalogs](docs/remote-skills.md).
The shared daemon checks upstream cheaply for changes and fetches a skill's
references and scripts when the skill is attached. `chauffeur skills` shows
their sources and freshness. No public registry search is enabled.

What it saves, measured with [`bench/`](bench) and
[`docs/plans/prompt-trimming.md`](docs/plans/prompt-trimming.md):

- **Longer benchmark tasks:** 29% fewer input tokens and 19% faster than plain
  OpenCode, with the same or better pass rate.
- **A session's first request:** 44–48% smaller (Opus 18.8k → 10.4k tokens,
  GPT-6 Luna 11.3k → 5.9k).
- **Every request:** about 480 tokens fewer from the shorter tool descriptions,
  with no loss in pass rate across 135 runs.

### Fewer permission prompts, and the right ones

- **Approves** what clearly serves your task: an edit in the project, a file
  you named, a sibling worktree of the same repository when the agent says why.
- **Asks you** about the rest: unrelated folders, credential stores such as
  `~/.ssh`, anything it is unsure of.
- **Blocks** irreversible harm (`rm -rf /`) and shell rewrites of project files
  (`sed -i`, `node -e`, heredocs), pointing the agent at the edit tools, so every
  change stays reviewable. A force-push to `main` always asks.

![Chauffeur approves task-related access outside the project; the edits and tests complete without a permission prompt](docs/media/approvals.gif)

![Chauffeur denying a sed rewrite; the agent switches to the edit tool](docs/media/guard.gif)

### Secrets stay out of the judge's context

Chauffeur redacts credential-shaped strings before sending context to Jev.
This protects the judge request, not OpenCode's own transcript. Here, the
right pane shows the user message extracted from the recorded Jev request.

![A fake key in the OpenCode prompt is replaced with REDACTED in the request sent to Jev](docs/media/redaction.gif)

### Steers when the agent drifts

After a tool call, Chauffeur nudges the agent back on course, and hands over the
matching skill when there is one. It steers away from:

- `cat` or `sed -n` instead of the read tool
- a recursive `grep` instead of `rg`
- a browser for GitHub, Slack, Jira, or X, where `gh`, `slackcli`, `jira`, or
  `twitter-cli` does it directly
- digging through a dependency's installed source (`node_modules`, the Cargo
  registry) before reading its documentation

Add your own rules:

- for every session, in `~/.config/chauffeur/rules/`
- for one project, in its `.chauffeur/rules/`

![The agent prints two files with cat; Chauffeur steers it to the read tool](docs/media/steer.gif)

![After a browser fetch of an X post, Chauffeur hands over twitter-cli and the agent switches to it](docs/media/drift.gif)

### Rulebooks: `/goal` and your own

A rulebook is a set of rules you switch on for one session with a slash command.

**`/goal <objective>`** keeps the agent working until the objective is met:

- The agent breaks the objective into todos with a `todowrite` tool. Chauffeur
  reads the list from the session; the tool comes from a todo plugin.
- It cannot finish while a todo is open, and it ends once the evidence shows
  the goal is met.
- While a todo is open it leaves the next nudge to the todo plugin, so an
  open todo is never reminded twice.
- It pauses when only you can unblock it.
- It stops after 20 continuations with a progress summary.

![/goal resumes unfinished work after one test fix, then verifies the complete passing suite](docs/media/goal.gif)

Add your own in `skills/rulebooks/`, or per project in `.chauffeur/rulebooks/`
([format](skills/README.md#rulebooks)).

Here, a project's `release.json` defines `/release`:

1. Bump the version.
2. Update the changelog.
3. Run the tests.
4. Commit and tag locally.
5. Check the evidence.

![A project rulebook becomes /release and checks that a local release is complete](docs/media/release.gif)

### Ask for what's missing

With **`ask_chauffeur`**, the agent can ask in plain words for a tool or skill
it lacks, then use what Chauffeur grants to finish the task.

![The agent asks Chauffeur for a Slack-posting skill and uses slackcli to finish the handoff](docs/media/ask.gif)

### Model routing: past usage limits, and cheaper long sessions

Chauffeur's model router keeps the agent on a model that can serve it, and
keeps long sessions from paying full price after a break.

**Failover.** A usage limit covers the whole provider account, so on a limit
Chauffeur:

- switches to the same tier on another provider you use directly (your
  Anthropic or OpenAI subscription), recommended models first, never an
  OpenCode copy of them;
- uses OpenCode Zen's free models only when nothing else is left;
- skips blacklisted models even when pinned; OpenCode Go is blocked by
  default (`"blacklist": ["opencode-go/*"]` in `model-router.json`);
- switches back once the limit has likely cleared.

![Opus hits a usage limit, then GPT-6.1 Sol does; Chauffeur switches each time and the task finishes on a free model](docs/media/failover.gif)

In this clip, a local endpoint stands in for Anthropic and returns a usage
limit. The GPT-6.1 Sol limit that follows is a real one.

**Warm caches, then a cheap summary.** When a session goes idle, Chauffeur
keeps its prompt cache warm, sending a keep-alive just before the provider's
cache would expire. It stops after an hour, or sooner once more keep-alives
would cost more than re-sending the conversation. When you come back after the
cache has expired, and the conversation is long (a share of the model's
window, between 50k and 100k tokens), a cheaper model summarizes it first.
Your prompt still runs on your own model.

![Chauffeur warms an idle session so the next prompt reads its cache, then compacts it once warming stops and the cache expires](docs/media/warming.gif)

The clip is a real Claude Sonnet 5.5 session with shortened limits (warming
for at most 5 minutes, summarizing from 8k tokens), driven and recorded with
[termctrl](https://github.com/anomalyco/terminal-control); the idle stretches
play at 80x. Measured in it, and in a second session with warming off:

| Moment | What Chauffeur did | What the provider billed |
|---|---|---|
| Prompt 2, 6½ min after the last reply | One keep-alive at 4 min, so no summary | 7,625 tokens read from cache, 148 written |
| The same prompt with warming off | Nothing | 4,940 read, 2,816 written: the conversation was re-sent |
| Prompt 3, 5½ min after the last keep-alive | GPT-6 Luna summarized in 5 s; the prompt ran on Sonnet | 0 read, 8,057 written: the cache had expired, as Chauffeur judged |

Leave OpenCode's own `warming` setting off, or sessions are warmed twice.

### And quietly

- **Cheaper subagents.** When the agent hands routine work (searching, reading,
  mechanical edits) to a subagent, Chauffeur steers it to a cheaper model from
  the same provider, such as Sonnet under Opus.
- **Questions with options.** When the agent ends its turn by asking you
  something in plain text, Chauffeur steers it to the question tool, so the
  ways forward arrive as options you can pick and the turn carries on.
- **Clean test sessions.** When the agent tests OpenCode by running it against
  your real data and finishes without deleting the sessions the test created,
  Chauffeur resumes it to delete them.
- **[sourcefed](https://github.com/StevenJPx2/sourcefed) events.** Chauffeur
  sets up monitors for the PR, Jira issue, or Slack thread you are working on,
  and lets through only the events the agent needs to act on.

  ![The agent opens a PR, Chauffeur has sourcefed watch it, and a real CI failure resumes the agent to fix and push](docs/media/monitor.gif)

  The repository (a throwaway private one) and its CI failure are real.
  Delivery of monitors Chauffeur creates mid-session needs sourcefed after
  0.3.4 ([28f4464](https://github.com/StevenJPx2/sourcefed/commit/28f4464)),
  which this clip runs.

## Install

You need:

- Rust
- Node.js
- OpenCode 2
- an API key for your chosen judge: TypeSafe (default) or SubQ

```sh
export TYPESAFE_API_KEY=...          # required for the default Jev judge
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
none is running.

### Choose a judge

Jev stays the default. To use [SubQ Snap](https://docs.subq.ai/endpoints/decisions-api/)
as a fallback, with both API keys in the daemon's environment:

```sh
export CHAUFFEUR_JUDGE_FALLBACK=subq
```

To use SubQ as the primary judge instead:

```sh
export CHAUFFEUR_JUDGE=subq
export CHAUFFEUR_JUDGE_FALLBACK=none
```

SubQ reads `SUBQ_API_KEY`. Restart the daemon after changing these variables.
Fallback happens only on failed calls or invalid answers, never to override a
valid low-confidence answer. A failed primary is retried after 60 seconds.
Each provider has a 3-second timeout. [Judge configuration](docs/judges.md)
covers reverse fallback, recording, and retention. Image inputs are not wired
into Chauffeur yet.

If every configured judge fails, Chauffeur stands aside:

- Prompts go through unchanged.
- Permission requests fall back to asking you.

## See what it decided

Every decision is logged, with the questions asked and how long they took:

```sh
chauffeur audit                   # the last 20 decisions
chauffeur audit 100
chauffeur audit --follow --brief  # new decisions as they happen, one short line each
```

![The live audit feed shows permission decisions and judgment times while OpenCode works](docs/media/audit.gif)

```text
00:16:40 UTC ses_f32a1b… user_message Check my latest Twitter mentions → attach_skills ["twitter-cli"] | 1 asked, 493 ms
00:16:42 UTC ses_f32a1b… permission_request shell rm -rf / → permission deny | 0 asked, 0 ms; vetoed: rm -rf /
```

## Configure

| Variable | Default |
|---|---|
| `TYPESAFE_API_KEY` | required |
| `CHAUFFEUR_IDLE_STEERING` | `false`; `true` lets turn-end rules and rulebooks resume the agent |
| `CHAUFFEUR_CONFIG_DIR` | `~/.config/chauffeur`; per-capability overrides as JSON, and your own `rules/` |
| `CHAUFFEUR_SKILLS_DIR` | `~/.config/chauffeur/skills` (a link to `skills/`) |
| `CHAUFFEUR_STATE_DIR` | `~/.local/state/chauffeur` (session memory, audit log) |
| `CHAUFFEUR_DAEMON_URL` | `http://127.0.0.1:18790` |
| `CHAUFFEUR_SOURCEFED` | on; `off` leaves sourcefed alone |
| `CHAUFFEUR_EMBEDDINGS` | `http://127.0.0.1:18791`; a loopback embedding service that ranks skills so each prompt judges the closest few, or `off` |

Rules, rulebooks, and permission contracts are strict JSON in [`skills/`](skills):

- [`skills/README.md`](skills/README.md) documents each format.
- `chauffeur skill validate PATH` checks a file.
- [`ARCHITECTURE.md`](ARCHITECTURE.md) covers the design.

The words are config too:

- Every question Chauffeur asks Jev and every message it shows lives in a
  `texts` object in [`skills/config/`](skills/config).
- The plugin's tool descriptions and replies live in
  [`skills/config/hosts/opencode.json`](skills/config/hosts/opencode.json).

To reword one, name only that text in your file of the same name:

```json
// ~/.config/chauffeur/hosts/opencode.json
{ "ask_chauffeur": { "nothing_found": "Nothing extra for that: use the tools you have." } }
```

Edits apply without a restart:

- Within a second, the daemon picks up rules, rulebooks, contracts, and wording
  in `skills/`, and your overrides in `~/.config/chauffeur/`. It keeps session
  memory and running rulebooks.
- Within five seconds, the plugin re-registers its tools with new wording.
- A file that fails to load, such as one naming a placeholder its text does not
  accept, leaves the previous version running and logs why.

![Saving a project rulebook makes its slash command appear without restarting](docs/media/reload.gif)

This clip uses a private, patched OpenCode 2.0.23 build. The stock release can
leave autocomplete stale because background registry updates omit their project
location. The [patch](docs/patches/opencode-2.0.23-command-location.patch) keeps
that location on command updates; your installed OpenCode is not modified by
Chauffeur.

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
repositories:

- The `jira`, `slackcli` and `twitter` CLIs are stubs, and Twitter replies are
  fixtures.
- The ask demo's `AGENTS.md` requires a Slack handoff and supplies the question
  the agent asks Chauffeur.
- The `/goal` demo's project policy allows one failing test file to be fixed per
  turn, making its real automatic continuation visible.
