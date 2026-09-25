# Chauffeur skills

Contracts and skills for Chauffeur, kept apart from its code. The daemon loads this
directory from `CHAUFFEUR_SKILLS_DIR` (default `~/.config/chauffeur/skills`, usually a
symlink to this folder). Contracts are strict JSON: unknown fields are
rejected, and `chauffeur skill validate PATH` checks one.

| Directory | What it holds |
|---|---|
| `permission/` | Permission contracts: which edits, shell commands, and outside directories OpenCode would ask about that Jev may approve. |
| `misuse/` | Misuse contracts: tool calls Jev checks after they run, with the steer message and an optional skill to hand over. |
| `handoff/` | Skills Chauffeur hands over, in OpenCode's `SKILL.md` format: `slack-cli`, `jira-cli`, and `twitter-cli`. Link them into a skills directory OpenCode reads, such as `~/.agents/skills`. |

Project-local skills live under each Git worktree's `.chauffeur/skills/`, with
optional deeper `.chauffeur/skills/` directories for sessions opened there.
They are separate from host-loaded `SKILL.md` files: a contract states when to
ask Jev, up to two dependent yes/no judgments, and a `resume` or `wait` message
when both judgments pass. The current workspace and tool-call history scope
the contract to its project. Use `chauffeur skill validate-project WORKSPACE`
to check all active files.

## Misuse contracts

```json
{
  "schema_version": 1,
  "identity": { "id": "slack-via-browser", "name": "Slack through a browser", "version": "1.0.0" },
  "match": { "tools": ["shell", "webfetch", "execute"] },
  "question": "Does this call use a browser … to read or send Slack messages, where slackcli does it directly?",
  "nudge_at_or_above": 0.7,
  "minimum_confidence": 0.4,
  "steer": "Use slackcli for Slack instead of the browser; the skill below shows how.",
  "handoff_skill": "slack-cli",
  "cooldown_seconds": 120
}
```

After each call to a watched tool, Chauffeur shows Jev the call's input and asks every
matching contract's `question` (phrase it as "Does this call …?"), all in one Jev request, and adds that a call the user explicitly
asked for does not count. A yes at or above `nudge_at_or_above`, with at least
`minimum_confidence`, steers the running turn with `steer` and delivers `handoff_skill`.
Each contract then rests for `cooldown_seconds` per agent. Code Mode tools are called
through `execute`, so watch `execute` to catch browser use there.

IDs are 1–64 characters of `[a-z0-9-]`; `question` and `steer` are at most 1 KiB; a
directory holds at most 64 contracts.
