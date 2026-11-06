# Rule files: v1 to v2

A v1 rule and a v2 rule say the same things: which event, which cheap
filter, which yes/no questions, and what to deliver. v2 names the judge and
its confidence, and every question is a step in a strategy.

## Field by field

| v1                                                                     | v2                                                                                                                                      |
| ---------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| `schema_version`, `id`, `name`                                         | The ID is the file's path under its rules folder, such as `cat-to-read` or `team/release`                                               |
| `"on": "tool_result"`                                                  | `"on": "tool.after"`                                                                                                                    |
| `"on": "turn_end"`                                                     | `"on": "response.finished"`, with `"when": { "reason": ["done"] }`                                                                      |
| `when.tools`                                                           | `when.tools`                                                                                                                            |
| `when.input_excludes`                                                  | `when.input.excludes`: skip a call whose input contains any of these                                                                    |
| `when.result_includes`                                                 | `when.result.contains`: admit a call only when its result contains one of these. The result decides admission and never reaches a judge |
| `when.session_kind`                                                    | `when.agent`: `"top-level"` or `"subagent"`                                                                                             |
| `when.history`, `tools_called`, `tools_called_any`, `tools_not_called` | `when.history`: `reach` (`"request"`, the default, or `"session"`), `called`, `called_any`, `not_called`                                |
| `when.status`, `when.source`, `when.hooks`                             | `when.history`: `status`, `sources`, `events` (such as `"github:merged"`)                                                               |
| A step's `question`                                                    | A step's `ask`: `"outcomes": ["yes", "no"]`, its `instructions`, and the `evidence` blocks it sees                                      |
| `yes_at_or_above`                                                      | The named judge's `confidence`. `then` acts on `yes`, so `0.7` reads as v1's 0.7                                                        |
| `minimum_confidence`                                                   | Not needed: with yes at 0.7 or more, Jev's margin is already 0.4 or more                                                                |
| A second step, asked once the first holds                              | `"after": [{ "step": "<first>", "when": ["yes"] }]`                                                                                     |
| `then.delivery`: `steer`, `resume`, `wait`                             | `message.append` with `"delivery"`: `"now"`, `"wake"`, `"next"`                                                                         |
| `then.text`                                                            | `"parts": [{ "text": "…" }]`                                                                                                            |
| None                                                                   | `"label"`: one line the harness shows as `Chauffeur · <rule>: <label>`, such as "commit message isn't conventional; …"                  |
| `then.skill`                                                           | `"parts": [{ "text": "…" }, { "skill": "jira-cli" }]`                                                                                   |
| `priority`                                                             | `priority`                                                                                                                              |
| `once`                                                                 | `"once": "request"` (again after the next user message) or `"session"`                                                                  |
| `cooldown_seconds`                                                     | `cooldown_seconds`, per rule and session                                                                                                |
| `rules.json` `max_deliveries: 2`                                       | An event appends at most two messages, highest `priority` first                                                                         |
| `CHAUFFEUR_IDLE_STEERING=true`                                         | `"wake": true` in `~/.config/chauffeur/settings.json`: a `wake` delivery is dropped without it                                          |

## Evidence

v1 rendered one situation text for every question. v2 hands a step only the
blocks it names. The adapter gathers them from the session:

| Event               | Blocks                                                                                                                                                               |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `tool.after`        | `tool_call` (the tool, its input, and whether it succeeded), `request` (the user's latest request), `activity` (recent agent activity, oldest first, at most 16 KiB) |
| `response.finished` | `summary` (the agent's closing message), `request`, `activity`, `todos`                                                                                              |
| `permission.asked`  | `permission` (the action, its resources, the agent's reason, the workspace), `request`, `activity`                                                                   |
| `prompt.sent`       | `message`, `previous` (the agent's previous prompt), `workspace`                                                                                                     |

A steer should not correct the agent for doing exactly what the user asked,
such as `cat package.json` when the user said so. v1 added "A call the user
explicitly asked for, exactly as asked, does not count" to every tool-result
question out of sight. In v2 each ported rule ends its instructions with that
sentence and names `request` in its evidence, so the rule file holds the
whole question the judge sees. A rule that should apply even to a call the
user asked for, such as a guard, leaves the sentence out.

## Worked example: `conventional-commits`

v1, `skills/rules/conventional-commits.json`:

```json
{
  "schema_version": 2,
  "id": "conventional-commits",
  "on": "tool_result",
  "when": {
    "tools": ["shell", "execute"],
    "input_excludes": ["feat:", "feat(", "fix:", "fix(", "…"]
  },
  "steps": [
    {
      "id": "nonconventional",
      "question": "Does this call create a git commit whose message does not follow …?",
      "yes_at_or_above": 0.7,
      "minimum_confidence": 0.4
    }
  ],
  "then": {
    "delivery": "steer",
    "text": "Chauffeur: Use Conventional Commits for commit messages, …"
  },
  "cooldown_seconds": 120
}
```

v2, `distribution/rules/conventional-commits.json`:

```json
{
  "on": "tool.after",
  "when": {
    "tools": ["shell", "execute"],
    "input": {
      "contains": "git commit",
      "excludes": ["feat:", "feat(", "fix:", "fix(", "…"]
    }
  },
  "judges": { "quick": { "tier": "system-one", "confidence": 0.7 } },
  "strategy": [
    {
      "id": "nonconventional",
      "judge": "quick",
      "ask": {
        "outcomes": ["yes", "no"],
        "evidence": ["tool_call", "request"],
        "instructions": "Does this call create a git commit whose message does not follow …? A call the user explicitly asked for, exactly as asked, does not count."
      }
    }
  ],
  "then": [
    {
      "if": { "nonconventional": "yes" },
      "message.append": {
        "label": "commit message isn't conventional; asking the agent to amend it",
        "parts": [
          { "text": "Chauffeur: Use Conventional Commits for commit messages, …" }
        ],
        "delivery": "now"
      }
    }
  ],
  "cooldown_seconds": 120
}
```

Instructions and texts carry over word for word, with a `label` added; the
pages for each rule abbreviate them as `…`.
