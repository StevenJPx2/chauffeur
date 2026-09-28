# Rulebooks: rule sets the user activates

A rulebook is a named set of rules that stays dormant until the user turns it
on in a session, with arguments, and off again. Rules today are always on
(shipped) or on per project (`.chauffeur/rules/`); a rulebook adds a third
scope: on for one session, by request.

"Skill" already names `SKILL.md` files (skill exposure, `ask_chauffeur`,
`then.skill`), so this plan calls the new unit a rulebook.

## Status

Steps 1–4 of the build order are built; the user-facing format is in
`skills/README.md`, and `skills/rulebooks/goal.json` ships the goal. Where the
build differs from the draft below:

- `on_start` and `on_budget` are plain texts; `budget` is a number of
  deliveries.
- The no-spin guard is state, not a gate fact: after the book resumes the
  agent, its turn-end rules wait for a tool call or a user message. No `*`
  wildcard was needed.
- `then.end` has `complete` and `pause`; the goal pauses when the agent says
  only the user can unblock it.
- Rulebooks are scoped: a shipped book may list `scope` folders, and a
  project keeps its own in `.chauffeur/rulebooks/`. OpenCode keeps commands
  per location, and the plugin runs once per location, so each location is
  offered only its books (`rulebooks` RPC with the workspace). `/ticket`
  (HPDP Overlay) replaced the project rules copied into its 26 worktrees.
- A session runs up to four books at once, one of each; a running book is a
  copy taken at start. The start text may be a Jira key, a Slack thread link,
  or plain text (`{input}`), and a book hands over its skills on start.
- Headless `opencode run` neither dispatches slash commands nor reports turn
  ends before it exits, so step 5 (a bench task) needs a harness that drives
  a live session; `/goal` is verified in the TUI.

## The target: Codex `/goal`

Codex's goal ([cookbook](https://developers.openai.com/cookbook/examples/codex/using_goals_in_codex))
is a thread-scoped completion contract:

- `/goal <objective>` sets it; `/goal`, `/goal pause`, `/goal resume`,
  `/goal clear` manage it.
- At a safe boundary (turn ended, thread idle, no queued user input) it checks
  the objective against evidence in the thread. Not done: inject a
  continuation prompt and keep working. Done: stop.
- If a continuation turn made no tool call, the next continuation is
  suppressed, so it cannot spin.
- A budget stops it; reaching the budget is not completion, and a budget
  prompt asks for a progress and blocker summary.
- Only the user or the system pauses, resumes, or clears it.

Each of those maps onto the rule engine:

| Goal piece | Chauffeur today | Needed |
|---|---|---|
| Check at turn end | `on: "turn_end"`, Jev steps | Evidence from the turn in the question |
| Continue | `then.delivery: "resume"` wakes the idle agent | – |
| Objective in the prompt | – | Arguments: `{args}` in questions and text |
| Thread-scoped, on by request | – | Activation per session |
| pause / resume / clear | – | Lifecycle state per session |
| Done ends it | – | A rule effect that deactivates the rulebook |
| No spinning | gate `tools_called_any` (turn window) | Gate relative to the last continuation |
| Budget | `cooldown_seconds`, `once` | A continuation count, then a budget rule |

## Format

`skills/rulebooks/<id>.json` (shipped) or `.chauffeur/rulebooks/<id>.json`
(project). Strict, like rules; the rules inside use the existing rule format
unchanged, plus `{args}` and one new `then` field.

```json
{
  "schema_version": 1,
  "id": "goal",
  "name": "Goal",
  "description": "Keep working until the objective is verified done.",
  "args": { "required": true, "max_bytes": 1024 },
  "budget": { "deliveries": 20 },
  "rules": [
    {
      "schema_version": 2,
      "id": "goal-continue",
      "name": "Goal: keep going",
      "on": "turn_end",
      "when": { "history": "turn", "tools_called_any": ["*"] },
      "steps": [
        {
          "id": "not-done",
          "question": "The user's goal is: {args}. Judging only from concrete evidence in the session (files changed, commands run, test or benchmark output), is the goal still not achieved and is there a next step the agent can take without the user?",
          "yes_at_or_above": 0.7,
          "minimum_confidence": 0.4
        }
      ],
      "then": {
        "delivery": "resume",
        "text": "Goal: {args}\nNot done yet. Check the goal against the evidence, pick the next most useful step, and continue. If you are blocked, stop and say what blocks you and what would unlock it."
      }
    },
    {
      "schema_version": 2,
      "id": "goal-done",
      "name": "Goal: done",
      "on": "turn_end",
      "steps": [
        {
          "id": "done",
          "question": "The user's goal is: {args}. Does concrete evidence in the session show the goal is achieved?",
          "yes_at_or_above": 0.8,
          "minimum_confidence": 0.5
        }
      ],
      "then": {
        "delivery": "wait",
        "text": "Goal achieved: {args}",
        "end": "complete"
      }
    }
  ],
  "on_budget": {
    "delivery": "resume",
    "text": "Goal budget reached: {args}\nStop substantive work. Summarise progress, the evidence, what blocks completion, and the next useful step."
  }
}
```

New pieces:

- `args`: whether the rulebook takes arguments, and their bound. `{args}` is
  substituted into step questions and `then.text` when the rulebook activates;
  the substituted text is bounded by the existing 1 KiB limits.
- `then.end`: `"complete"` deactivates the rulebook after delivering. Only a
  rulebook rule may set it.
- `budget.deliveries`: the rulebook's rules deliver at most this many times per
  activation; the next delivery would be `on_budget` instead, after which the
  rulebook stops as `budget_limited`.
- Gate facts stay exact. The no-spin guard is `tools_called_any` in the turn
  window, where a turn now also starts at a Chauffeur resume, not only a user
  message, so "the continuation turn made a tool call" is a fact, not a Jev
  question. (Needs a `*` wildcard or a `tools_called_count` fact.)

## Lifecycle

Per session, in the rules capability's saved state (so a daemon restart keeps
it), keyed by `(agent, rulebook)`:

`active(args, deliveries) → paused → active → complete | budget_limited | cleared`

Only the user changes state, except `then.end` and the budget, matching Codex:
the model cannot clear or pause a goal. A user message does not end a goal;
it renews the turn window as today.

Ordering at a turn end: `goal-done` before `goal-continue` (priority), and at
most one of a rulebook's rules delivers per signal, so the agent is never told
both "done" and "keep going".

## Protocol and adapter

- New signal `rulebook` from the host: `{ command: "start" | "pause" |
  "resume" | "clear" | "status", rulebook, args }`. The engine answers with a
  context effect confirming the state ("Goal set: …"), so the adapter needs no
  rulebook knowledge.
- The OpenCode adapter registers one slash command per loaded rulebook through
  `command.transform` (`editor.add({ name, description, execute })`); `execute`
  receives the session and the raw argument text. `/goal pause`, `/goal
  resume`, `/goal clear` and bare `/goal` map to the lifecycle commands; any
  other text starts the rulebook with those arguments.
- `/goal <objective>` should also start work, like Codex: the confirmation is
  delivered with `resume`, carrying the objective, so the agent begins.

## Evidence

`turn_end` carries only the workspace and the latest user request. A goal
check needs the turn's evidence, so `TurnEnd` gains a bounded `summary`: the
agent's final message (clipped), the tools it called this turn, and whether
the last check commands succeeded. Questions for a rulebook rule include it.
This helps shipped turn-end rules too.

## Limits

- One active rulebook per session at first; more later if a second use needs
  it.
- A continuation is at most one `resume` per turn end, so the idle-steering
  guard (`CHAUFFEUR_IDLE_STEERING`) must be on for rulebooks to continue;
  status says so when it is off.
- The budget counts deliveries, not tokens: Chauffeur does not see token
  counts. A token budget can come later from the host's step events.

## Build order

1. `{args}`, `then.end`, and the rulebook file format with validation and
   tests (rules crate).
2. Per-session activation and lifecycle state, saved with the rules state;
   rulebook rules join `rules_for` only while active.
3. The `rulebook` signal and its confirmation effect; `TurnEnd.summary`.
4. OpenCode slash commands from the loaded rulebooks.
5. Ship `goal.json`; add a bench task where the first turn usually stops
   short (a long task with visible progress checks) and compare `full` with
   and without `/goal`.
