# Chauffeur skills

Contracts and skills for Chauffeur, kept apart from its code. The daemon loads this
directory from `CHAUFFEUR_SKILLS_DIR` (default `~/.config/chauffeur/skills`, usually a
symlink to this folder). Contracts are strict JSON: unknown fields are
rejected, and `chauffeur skill validate PATH` checks one.

| Directory | What it holds |
|---|---|
| `permission/` | Permission contracts: which edits, shell commands, and outside directories OpenCode would ask about that Jev may approve. |
| `rules/` | Rules: steers after a tool call, with an optional skill to hand over, and idle reminders at a turn end. |
| `rulebooks/` | Rulebooks: named sets of rules the user starts in a session with a slash command, such as `/goal <objective>`. |
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
  end, Jev sees the user's latest request, the agent's closing message, and
  the tools it called since that request.
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

## Rulebooks

A rulebook is a set of rules that stays off until the user starts it in a
session. OpenCode offers each one as a slash command named after its `id`:
`/goal <text>` starts it with that text as its arguments; `/goal pause`,
`/goal resume`, and `/goal clear` control it; bare `/goal` reports its status.
A session runs up to four rulebooks at once, one of each, such as a `/goal`
inside a `/ticket`; each keeps its own budget and delivers at most one of its
rules per turn. Starting a running book again restarts it.

A rulebook is offered where it applies: a shipped one everywhere, or only
under its `scope` folders; a project's own, from `.chauffeur/rulebooks/` in
the worktree (read from its Git root down, like project rules), only there.
`/ticket` in `rulebooks/ticket.json` is scoped to the HPDP Overlay folders.

```json
{
  "schema_version": 1,
  "id": "goal",
  "name": "Goal",
  "description": "Keep working across turns until evidence shows the objective is done.",
  "args": { "required": true },
  "budget": 20,
  "on_start": "Goal: {args}\nWork toward this goal until …",
  "on_budget": "Goal budget reached: {args}\nSummarise …",
  "otherwise": "Goal: {args}\nNot done yet …",
  "rules": [ { "schema_version": 2, "id": "goal-done", "on": "turn_end", "…": "…",
               "then": { "delivery": "wait", "text": "Goal achieved: {args}", "end": "complete" } } ]
}
```

- **`scope`** (optional): up to 8 absolute or `~/` folders, each optionally
  ending in `/**`; the book is offered only in sessions under one of them.
- **`skills`** and **`input_skills`** (optional): skills handed over when the
  book starts or resumes, plus one per kind of input: `{ "jira": "jira-cli",
  "slack": "slack-cli" }`.
- **Input**: the start text is a Jira key (`ADEPT-12345`), a Slack thread link,
  or anything else. `{args}` is the text as typed; `{input}` describes it:
  "Jira ticket ADEPT-12345", "Slack thread https://…", or the text itself.
- **`rules`**: 1–8 rules in the rule format above, each ID starting with the
  rulebook's ID and `-`. `{args}` and `{input}` in a question or text are
  filled in on start. A rulebook delivers at most one of its rules per signal, highest
  `priority` first, so the agent never hears "done" and "keep going" together.
- **`then.end`** (rulebook turn-end rules only): `complete` stops the book
  after delivering; `pause` pauses it until `/<id> resume`.
- **`budget`**: deliveries per start, 1–200. The next one delivers
  `on_budget` instead, waking the agent, and stops the book.
- **`on_start`**: delivered, waking the agent, when the book starts or resumes.
  `/goal` shows it as a prompt, so the goal is visible in the session.
- **`otherwise`** (optional): delivered, waking the agent, at a turn end where
  the book's turn-end rules were asked and none holds. The goal uses it for
  "not done yet, keep going", so a goal never sits silent between "done" and
  "blocked". It counts against the budget.
- After a rulebook resumes the agent, its turn-end rules wait until the agent
  calls a tool or the user writes, so a book cannot keep waking an agent that
  does nothing. Turn-end rules need `CHAUFFEUR_IDLE_STEERING=true`.

The running book, its arguments, and its spent budget are saved with the
daemon's state, so a restart keeps them.
