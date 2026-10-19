---
name: chauffeur
description: Operate, configure, and debug Chauffeur, the daemon that judges an agent's prompts, permissions, and tool calls. Use when a skill, rule, rulebook, or permission decision is wrong or missing; to write a rule or rulebook; to add a remote skill source; to read the audit log; or to restart the daemon. Not for general OpenCode setup.
---

# Chauffeur

Chauffeur is a daemon beside the coding agent. The harness plugin reports what
happens (prompts, tool calls, permission requests, turn ends) as signals; the
daemon asks a fast judge yes/no questions and answers with effects: attach a
skill, hide or reveal tools, allow or ask about a permission, steer or resume
the agent, switch models. The plugin only relays; all behaviour lives in the
daemon, so another harness needs no logic of its own.

Repository: <https://github.com/StevenJPx2/chauffeur>. Its `README.md`,
`ARCHITECTURE.md`, `skills/README.md`, and `docs/remote-skills.md` are the
authoritative references; read the relevant one before changing behaviour.

## First, look at what it decided

Every decision is logged with the questions asked and their timing. Start here
before guessing why something happened:

```sh
chauffeur audit 30            # the last 30 decisions
chauffeur audit --follow --brief
chauffeur health              # is the daemon up, and which version
chauffeur skills              # remote skill catalog and each source's freshness
```

The raw log is `~/.local/state/chauffeur/audit.jsonl` (rotated to `audit.1.jsonl`).
Each line names the signal, the questions, the judge's answers, and the
effects. A low answer under a rule's bar explains a rule that did not fire.

## Where things live

| What | Where |
|---|---|
| Your overrides and personal rules | `~/.config/chauffeur/` (`rules/`, `<capability>.json`, `hosts/opencode.json`) |
| Remote skill sources | `~/.config/chauffeur/skill-sources.json` |
| Shipped rules, rulebooks, contracts, defaults | `skills/` in the repository, linked at `~/.config/chauffeur/skills` |
| A project's own rules and rulebooks | `.chauffeur/rules/`, `.chauffeur/rulebooks/` in its worktree |
| Session memory, audit log, skill cache | `~/.local/state/chauffeur/` |

Edits to any of these reload within a second without a restart. A file that
fails to load leaves the previous version running and logs why.

## Change behaviour

- **A skill should or should not attach:** its `description` decides. The judge
  reads only its first 400 bytes, so lead with when to use it. For a remote
  skill, set your wording under `descriptions` in `skill-sources.json` instead
  of editing the cached copy.
- **A skill is never even asked about:** each prompt judges only pinned skills
  plus the 16 closest by meaning, ranked by the local embedding service. A
  skill triggered by the kind of work rather than its topic (a coding
  discipline, a docs-first habit) belongs in `ranking.always` in
  `~/.config/chauffeur/skill-exposure.json`. The audit log lists the questions
  each prompt asked.
- **A judgment bar:** override only that field in
  `~/.config/chauffeur/<capability>.json`, such as
  `{ "needed": { "at": 0.8 } }` in `skill-exposure.json`. Unknown fields stop
  the daemon with an error naming them.
- **A steer or reminder:** add a rule to `~/.config/chauffeur/rules/` (all
  projects) or `.chauffeur/rules/` (one project). Formats are in
  `skills/README.md`. Phrase tool-result questions as "Does this call …?" and
  ask only what the judge can see in the call.
- **A slash-command workflow:** add a rulebook (`/goal` is the example).
- **Check a file before relying on it:**

```sh
chauffeur skill validate ~/.config/chauffeur/rules/my-rule.json
chauffeur skill validate-project /path/to/worktree
```

- **Probe a rule without the agent:** write a signal JSON with a made-up
  `agent_id` and send it; the reply lists the effects it produced.

```sh
chauffeur signal --file /tmp/probe.json
```

## Remote skills

Skills listed in `~/.config/chauffeur/skill-sources.json` stay upstream. The
daemon checks each source once a day (one small request, nothing downloaded
when unchanged) and fetches a skill's references and scripts only when the
skill is attached. The attached text lists every file by full path for that
revision. A remote skill shadows a local one of the same name. See
`docs/remote-skills.md` for formats, bounds, and failure behaviour.

To find new ones, search the Skilld registry and add a skill only when the
user approves it:

```sh
chauffeur skill discover WORDS...                 # candidates; adds nothing
chauffeur skill approve OWNER/REPOSITORY/SKILL --dry-run
chauffeur skill approve OWNER/REPOSITORY/SKILL    # writes skill-sources.json
```

Approval asks the daemon's judges whether each skill is safe. Show the user
the dry run's author, license, path, behaviors, and review first. Never pass
`--allow`, `--accept-risk`, or `--unvetted` unless the user accepted the
behaviors, the unsafe verdict, or the missing verdict it overrides.

## Restart or update

The macOS LaunchAgent `xyz.stjhn.chauffeur` runs the daemon; the OpenCode
plugin starts one when none answers and replaces a daemon of another version.

```sh
launchctl kickstart -k gui/$(id -u)/xyz.stjhn.chauffeur      # restart the daemon
cargo install --locked --path crates/cli                      # update the binary
cd adapters/opencode && npm run deploy && opencode service restart   # update the plugin
```

Restart only when the binary or plugin changed: configuration and rules reload
on their own.

## Develop

Keep adapters thin: behaviour, policy, and state belong in the Rust crates.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cd adapters/opencode && npm run check
```
