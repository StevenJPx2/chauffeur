# Steering at the end of a turn

Each v1 `turn_end` rule is a v2 rule on `response.finished` that delivers
with `wake`, so the agent carries on without waiting for the user. A `wake`
delivery needs `"wake": true` in `~/.config/chauffeur/settings.json`, as v1's
needed `CHAUFFEUR_IDLE_STEERING=true`. The format mapping is in
[Rule files: v1 to v2](rule-format.md).

| Rule | Resumes the agent when | Slice |
|---|---|---|
| [`ask-with-question-tool`](#ask-with-question-tool) | Its closing message asks the user something in prose | 5 |
| [`clean-up-opencode-test-sessions`](#clean-up-opencode-test-sessions) | An OpenCode test left sessions in the real database | 5 |
| [`git-conflict-loop`](#git-conflict-loop) | It is stuck in a merge or rebase conflict | 5 |
| [`jira-transition-after-merge`](#jira-transition-after-merge) | Its PR merged and the Jira issue was not moved | 5 |

## `ask-with-question-tool`

Only top-level sessions: a subagent's question goes to its parent, not the user.

v1:

```json
{
  "on": "turn_end",
  "when": {
    "session_kind": "top_level"
  },
  "steps": [
    {
      "id": "asked-in-prose",
      "question": "Does the agent's closing message leave something for the user to decide or supply, …?",
      "yes_at_or_above": 0.7,
      "minimum_confidence": 0.4
    }
  ],
  "then": {
    "delivery": "resume",
    "text": "Chauffeur: Put that question to the user with the question tool, …"
  },
  "cooldown_seconds": 900
}
```

v2, `distribution/rules/ask-with-question-tool.json`:

```json
{
  "on": "response.finished",
  "when": {
    "reason": [
      "done"
    ],
    "agent": "top-level"
  },
  "judges": {
    "quick": {
      "tier": "system-one",
      "confidence": 0.7
    }
  },
  "strategy": [
    {
      "id": "asked-in-prose",
      "judge": "quick",
      "ask": {
        "outcomes": [
          "yes",
          "no"
        ],
        "evidence": [
          "summary",
          "request"
        ],
        "instructions": "Does the agent's closing message leave something for the user to decide or supply, …?"
      }
    }
  ],
  "then": [
    {
      "if": {
        "asked-in-prose": "yes"
      },
      "message.append": {
        "parts": [
          {
            "text": "Chauffeur: Put that question to the user with the question tool, …"
          }
        ],
        "delivery": "wake"
      }
    }
  ],
  "cooldown_seconds": 900
}
```
## `clean-up-opencode-test-sessions`

Two questions: the second runs only when the first finds a test.

v1:

```json
{
  "on": "turn_end",
  "when": {
    "session_kind": "top_level"
  },
  "steps": [
    {
      "id": "opencode-test",
      "question": "Did the agent run OpenCode itself as a test this turn, …?",
      "yes_at_or_above": 0.7
    },
    {
      "id": "sessions-left",
      "question": "Judging by the recent activity, has the agent not yet deleted the sessions that test created …?",
      "yes_at_or_above": 0.7
    }
  ],
  "then": {
    "delivery": "resume",
    "text": "Chauffeur: Your OpenCode test left its sessions behind. …"
  },
  "cooldown_seconds": 600
}
```

v2, `distribution/rules/clean-up-opencode-test-sessions.json`:

```json
{
  "on": "response.finished",
  "when": {
    "reason": [
      "done"
    ],
    "agent": "top-level"
  },
  "judges": {
    "quick": {
      "tier": "system-one",
      "confidence": 0.7
    }
  },
  "strategy": [
    {
      "id": "opencode-test",
      "judge": "quick",
      "ask": {
        "outcomes": [
          "yes",
          "no"
        ],
        "evidence": [
          "activity"
        ],
        "instructions": "Did the agent run OpenCode itself as a test this turn, …?"
      }
    },
    {
      "id": "sessions-left",
      "judge": "quick",
      "after": [
        {
          "step": "opencode-test",
          "when": [
            "yes"
          ]
        }
      ],
      "ask": {
        "outcomes": [
          "yes",
          "no"
        ],
        "evidence": [
          "activity"
        ],
        "instructions": "Judging by the recent activity, has the agent not yet deleted the sessions that test created …?"
      }
    }
  ],
  "then": [
    {
      "if": {
        "sessions-left": "yes"
      },
      "message.append": {
        "parts": [
          {
            "text": "Chauffeur: Your OpenCode test left its sessions behind. …"
          }
        ],
        "delivery": "wake"
      }
    }
  ],
  "cooldown_seconds": 600
}
```
## `git-conflict-loop`

Reads the whole session's activity, and fires once per session.

v1:

```json
{
  "on": "turn_end",
  "when": {
    "history": "session",
    "status": [
      "implementing"
    ]
  },
  "steps": [
    {
      "id": "stuck",
      "question": "The agent is stuck repeating git conflict-resolution commands without making progress.",
      "yes_at_or_above": 0.8
    }
  ],
  "then": {
    "delivery": "resume",
    "text": "📋 Reminder (Git conflict loop detection): …"
  },
  "priority": 100,
  "once": true,
  "cooldown_seconds": 30
}
```

v2, `distribution/rules/git-conflict-loop.json`:

```json
{
  "on": "response.finished",
  "when": {
    "reason": [
      "done"
    ],
    "history": {
      "reach": "session",
      "status": [
        "implementing"
      ]
    }
  },
  "judges": {
    "sure": {
      "tier": "system-one",
      "confidence": 0.8
    }
  },
  "strategy": [
    {
      "id": "stuck",
      "judge": "sure",
      "ask": {
        "outcomes": [
          "yes",
          "no"
        ],
        "evidence": [
          "activity"
        ],
        "instructions": "The agent is stuck repeating git conflict-resolution commands without making progress."
      }
    }
  ],
  "then": [
    {
      "if": {
        "stuck": "yes"
      },
      "message.append": {
        "parts": [
          {
            "text": "📋 Reminder (Git conflict loop detection): …"
          }
        ],
        "delivery": "wake"
      }
    }
  ],
  "priority": 100,
  "once": "session",
  "cooldown_seconds": 30
}
```
## `jira-transition-after-merge`

The filter does the work: a Jira session, a merged PR, and no transition yet. Jev only confirms the merge from the activity.

v1:

```json
{
  "on": "turn_end",
  "when": {
    "history": "session",
    "source": [
      "jira"
    ],
    "hooks": [
      "github:merged"
    ],
    "tools_not_called": [
      "jira_transition_issue"
    ]
  },
  "steps": [
    {
      "id": "merged",
      "question": "The pull request has been merged.",
      "yes_at_or_above": 0.7
    }
  ],
  "then": {
    "delivery": "resume",
    "text": "📋 Reminder (Transition Jira after PR merge): …"
  },
  "priority": 20,
  "once": true,
  "cooldown_seconds": 30
}
```

v2, `distribution/rules/jira-transition-after-merge.json`:

```json
{
  "on": "response.finished",
  "when": {
    "reason": [
      "done"
    ],
    "history": {
      "reach": "session",
      "sources": [
        "jira"
      ],
      "events": [
        "github:merged"
      ],
      "not_called": [
        "jira_transition_issue"
      ]
    }
  },
  "judges": {
    "quick": {
      "tier": "system-one",
      "confidence": 0.7
    }
  },
  "strategy": [
    {
      "id": "merged",
      "judge": "quick",
      "ask": {
        "outcomes": [
          "yes",
          "no"
        ],
        "evidence": [
          "activity"
        ],
        "instructions": "The pull request has been merged."
      }
    }
  ],
  "then": [
    {
      "if": {
        "merged": "yes"
      },
      "message.append": {
        "parts": [
          {
            "text": "📋 Reminder (Transition Jira after PR merge): …"
          }
        ],
        "delivery": "wake"
      }
    }
  ],
  "priority": 20,
  "once": "session",
  "cooldown_seconds": 30
}
```
## Where `history` comes from

`when.history` reads facts the adapter sends with every event: the tools
called since the user's latest request and in the whole session, the
integration sources and events the session has seen (`github:merged` from a
sourcefed event), and its status (`implementing`, or `in_review` once a PR
was opened or a GitHub event arrived). The daemon keeps no session state, so
these facts travel with the event.
