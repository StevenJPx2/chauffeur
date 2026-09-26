# Chauffeur skills

Contracts and skills for Chauffeur, kept apart from its code. The daemon loads this
directory from `CHAUFFEUR_SKILLS_DIR` (default `~/.config/chauffeur/skills`, usually a
symlink to this folder). Contracts are strict JSON: unknown fields are
rejected, and `chauffeur skill validate PATH` checks one.

| Directory | What it holds |
|---|---|
| `permission/` | Permission contracts: which edits, shell commands, and outside directories OpenCode would ask about that Jev may approve. |
| `rules/` | Rules: steers after a tool call, with an optional skill to hand over, and idle reminders at a turn end. |
| `safety/` | The backstop's deny and confirm patterns, redaction shapes, and the bars for learning from them. |
| `config/` | Each capability's tunable defaults: judgment bars, budgets, timing, and word lists, plus `providers/` model tier tables. |
| `handoff/` | Skills Chauffeur hands over, in OpenCode's `SKILL.md` format: `slack-cli`, `jira-cli`, and `twitter-cli`. Link them into a skills directory OpenCode reads, such as `~/.agents/skills`. |

The files in `safety/` and `config/` are compiled into the daemon as its
defaults. To change one, put a file with the same name in
`CHAUFFEUR_CONFIG_DIR` (default `~/.config/chauffeur`), such as
`~/.config/chauffeur/skill-exposure.json` or
`~/.config/chauffeur/providers/anthropic.json`, holding only what you change:
objects merge field by field, and a list or value replaces the shipped one
whole. `backstop.json` and `redaction.json` add their lists to the shipped
ones instead, unless they set `"replace": true`. A misspelled field, or a bar outside `[0, 1]`, stops the daemon with
an error naming it. For example, to attach a skill only on a surer yes:

```json
{ "needed": { "at": 0.8 } }
```

A project keeps its own rules, in the same format, under each Git worktree's
`.chauffeur/rules/`, with optional deeper `.chauffeur/rules/` directories for
sessions opened there. The current workspace and tool-call history scope them
to their project. Use `chauffeur skill validate-project WORKSPACE` to check
every rule a session there would load.

## Rules

```json
{
  "schema_version": 2,
  "id": "slack-via-browser",
  "name": "Slack through a browser",
  "on": "tool_result",
  "when": { "tools": ["shell", "webfetch", "execute"] },
  "steps": [
    {
      "id": "misuse",
      "question": "Does this call use a browser … to read or send Slack messages, where slackcli does it directly?",
      "yes_at_or_above": 0.7,
      "minimum_confidence": 0.4
    }
  ],
  "then": {
    "delivery": "steer",
    "text": "Chauffeur: Use slackcli for Slack instead of the browser; the skill below shows how.",
    "skill": "slack-cli"
  },
  "cooldown_seconds": 120
}
```

- **`on`**: `tool_result` or `turn_end`. After a call to a tool in `when.tools`,
  Jev sees the call's input and is told that a call the user explicitly asked
  for does not count; phrase the question as "Does this call …?". At a turn
  end, Jev sees the user's latest request.
- **`when`**: exact facts checked before Jev is asked. `tools_called`,
  `tools_called_any`, and `tools_not_called` read the tools the agent ran
  since the latest user message (`"history": "turn"`, the default) or this
  session (`"history": "session"`); `status`, `source`, and `hooks`
  (`github:merged`) read the session.
- **`steps`**: one or two yes/no questions. A step holds at P(yes) ≥
  `yes_at_or_above` (0.5–1) with at least `minimum_confidence`; the second is
  asked only after the first holds.
- **`then`**: `steer` for a tool result, `resume` or `wait` at a turn end;
  `text`, an optional `label` (the name by default), and an optional `skill`.
- **`priority`**, **`once`**, **`cooldown_seconds`**: a signal delivers at
  most two rules, highest priority first. `once` renews at the next user
  message for a `turn` rule and never for a `session` rule.

IDs are 1–64 characters of `[a-z0-9_-]`; questions and text are at most 1 KiB; a
directory holds at most 64 rules, a project at most 32.
