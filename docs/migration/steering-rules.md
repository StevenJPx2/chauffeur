# Steering after a tool call

Each v1 `tool_result` rule is a v2 rule on `tool.after`. The format
mapping is in [Rule files: v1 to v2](rule-format.md); instructions and texts
carry over word for word and are abbreviated here as `…`.

| Rule                                                                                                  | Steers away from                                             | Slice                         |
| ----------------------------------------------------------------------------------------------------- | ------------------------------------------------------------ | ----------------------------- |
| [`conventional-commits`](rule-format.md#worked-example-conventional-commits)                          | A commit message outside Conventional Commits                | 1, with `input.excludes` in 5 |
| [`cat-to-read`](#cat-to-read)                                                                         | Printing project files through the shell                     | 5                             |
| [`grep-recursive`](#grep-recursive)                                                                   | A shell search of files, which the grep tool does            | 5                             |
| [`hand-rolled-patch`](#hand-rolled-patch)                                                             | Changing project files by script                             | 5                             |
| [`docs-before-source`](#docs-before-source)                                                           | Reading a dependency's source before its docs                | 5                             |
| [`prefer-cf-over-wrangler`](#prefer-cf-over-wrangler)                                                 | Wrangler where cf or the Cloudflare MCP does it              | 5                             |
| [`cheaper-subagent`](#cheaper-subagent)                                                               | Routine subagents on the top model                           | 5                             |
| [`github-via-browser`](#github-via-browser)                                                           | A browser for GitHub                                         | 5                             |
| [`jira-via-browser`, `slack-via-browser`, `twitter-via-browser`](#jira-slack-and-x-through-a-browser) | A browser for Jira, Slack or X, handing over the CLI's skill | 6                             |

## `cat-to-read`

The read guard in [permissions](permission.md) blocks these calls before they
run; this rule is the reminder after one that got through.

v1:

```json
{
  "on": "tool_result",
  "when": {
    "tools": ["shell"]
  },
  "steps": [
    {
      "id": "misuse",
      "question": "Did this call show the agent the text of a local project file, …?",
      "yes_at_or_above": 0.65,
      "minimum_confidence": 0.4
    }
  ],
  "then": {
    "delivery": "steer",
    "text": "Chauffeur: Read project files with the read tool, …"
  },
  "cooldown_seconds": 120
}
```

v2, `distribution/rules/cat-to-read.json`:

```json
{
  "on": "tool.after",
  "when": {
    "tools": ["shell"]
  },
  "judges": {
    "quick": {
      "tier": "system-one",
      "confidence": 0.65
    }
  },
  "strategy": [
    {
      "id": "misuse",
      "judge": "quick",
      "ask": {
        "outcomes": ["yes", "no"],
        "evidence": ["tool_call", "request"],
        "instructions": "Did this call show the agent the text of a local project file, …? A call the user explicitly asked for, exactly as asked, does not count."
      }
    }
  ],
  "then": [
    {
      "if": {
        "misuse": "yes"
      },
      "message.append": {
        "parts": [
          {
            "text": "Chauffeur: Read project files with the read tool, …"
          }
        ],
        "delivery": "now"
      }
    }
  ],
  "cooldown_seconds": 120
}
```

## `grep-recursive`

v1:

```json
{
  "on": "tool_result",
  "when": {
    "tools": ["shell"]
  },
  "steps": [
    {
      "id": "misuse",
      "question": "Does this call search the contents of local files from the shell, which the grep tool does? YES: grep -r or grep -R anywhere in the command, or rg, grep or git grep given file or folder paths to search (source, config, docs, or a dependency's files), even when head or tail trims the matches. NO when rg or grep only filter another command's output (after a pipe, such as cargo test | rg FAILED or ps | grep node); when it searches a log file; when the result only drives the script (rg -q, or a match count a loop tests); for rg --files and other listings; and for commands that search nothing.",
      "yes_at_or_above": 0.7,
      "minimum_confidence": 0.4
    }
  ],
  "then": {
    "delivery": "steer",
    "text": "Chauffeur: Search file contents with the grep tool, …"
  },
  "cooldown_seconds": 120
}
```

v2, `distribution/rules/grep-recursive.json`:

```json
{
  "on": "tool.after",
  "when": {
    "tools": ["shell"]
  },
  "judges": {
    "quick": {
      "tier": "system-one",
      "confidence": 0.7
    }
  },
  "strategy": [
    {
      "id": "misuse",
      "judge": "quick",
      "ask": {
        "outcomes": ["yes", "no"],
        "evidence": ["tool_call", "request"],
        "instructions": "Does this call search the contents of local files from the shell, which the grep tool does? YES: grep -r or grep -R anywhere in the command, or rg, grep or git grep given file or folder paths to search (source, config, docs, or a dependency's files), even when head or tail trims the matches. NO when rg or grep only filter another command's output (after a pipe, such as cargo test | rg FAILED or ps | grep node); when it searches a log file; when the result only drives the script (rg -q, or a match count a loop tests); for rg --files and other listings; and for commands that search nothing. A call the user explicitly asked for, exactly as asked, does not count."
      }
    }
  ],
  "then": [
    {
      "if": {
        "misuse": "yes"
      },
      "message.append": {
        "parts": [
          {
            "text": "Chauffeur: Search file contents with the grep tool, …"
          }
        ],
        "delivery": "now"
      }
    }
  ],
  "cooldown_seconds": 120
}
```

## `hand-rolled-patch`

v1:

```json
{
  "on": "tool_result",
  "when": {
    "tools": ["shell"]
  },
  "steps": [
    {
      "id": "misuse",
      "question": "Did this call write or change a project file … by script, …?",
      "yes_at_or_above": 0.7,
      "minimum_confidence": 0.4
    }
  ],
  "then": {
    "delivery": "steer",
    "text": "Chauffeur: Change files with the edit, write, or patch tools, …"
  },
  "cooldown_seconds": 120
}
```

v2, `distribution/rules/hand-rolled-patch.json`:

```json
{
  "on": "tool.after",
  "when": {
    "tools": ["shell"]
  },
  "judges": {
    "quick": {
      "tier": "system-one",
      "confidence": 0.7
    }
  },
  "strategy": [
    {
      "id": "misuse",
      "judge": "quick",
      "ask": {
        "outcomes": ["yes", "no"],
        "evidence": ["tool_call", "request"],
        "instructions": "Did this call write or change a project file … by script, …? A call the user explicitly asked for, exactly as asked, does not count."
      }
    }
  ],
  "then": [
    {
      "if": {
        "misuse": "yes"
      },
      "message.append": {
        "parts": [
          {
            "text": "Chauffeur: Change files with the edit, write, or patch tools, …"
          }
        ],
        "delivery": "now"
      }
    }
  ],
  "cooldown_seconds": 120
}
```

## `docs-before-source`

The second question runs only after the first says yes, so most calls cost
one judgment. It reads `activity`, not the call.

v1:

```json
{
  "on": "tool_result",
  "when": {
    "tools": ["read", "grep", "glob", "shell"]
  },
  "steps": [
    {
      "id": "dependency-source",
      "question": "Does this call read or search a third-party dependency's installed source …?",
      "yes_at_or_above": 0.7
    },
    {
      "id": "docs-unread",
      "question": "Judging by the recent activity, has the agent not yet looked up this library's documentation …?",
      "yes_at_or_above": 0.7
    }
  ],
  "then": {
    "delivery": "steer",
    "text": "Chauffeur: Check the library's documentation before its source: …"
  },
  "once": true
}
```

v2, `distribution/rules/docs-before-source.json`:

```json
{
  "on": "tool.after",
  "when": {
    "tools": ["read", "grep", "glob", "shell"]
  },
  "judges": {
    "quick": {
      "tier": "system-one",
      "confidence": 0.7
    }
  },
  "strategy": [
    {
      "id": "dependency-source",
      "judge": "quick",
      "ask": {
        "outcomes": ["yes", "no"],
        "evidence": ["tool_call", "request"],
        "instructions": "Does this call read or search a third-party dependency's installed source …?"
      }
    },
    {
      "id": "docs-unread",
      "judge": "quick",
      "after": [
        {
          "step": "dependency-source",
          "when": ["yes"]
        }
      ],
      "ask": {
        "outcomes": ["yes", "no"],
        "evidence": ["activity"],
        "instructions": "Judging by the recent activity, has the agent not yet looked up this library's documentation in this session …?"
      }
    }
  ],
  "then": [
    {
      "if": {
        "docs-unread": "yes"
      },
      "message.append": {
        "parts": [
          {
            "text": "Chauffeur: Check the library's documentation before its source: …"
          }
        ],
        "delivery": "now"
      }
    }
  ],
  "once": "request"
}
```

## `prefer-cf-over-wrangler`

v1:

```json
{
  "on": "tool_result",
  "when": {
    "tools": ["shell", "execute"],
    "input_excludes": [
      "cf ",
      " cf",
      "\"cf\"",
      "'cf'",
      "wrangler tail",
      "wrangler secret"
    ]
  },
  "steps": [
    {
      "id": "misuse",
      "question": "Does this call use the Wrangler CLI … where the cf CLI or a Cloudflare MCP tool can do it directly? …",
      "yes_at_or_above": 0.7
    }
  ],
  "then": {
    "delivery": "steer",
    "text": "Chauffeur: Prefer the cf CLI or a Cloudflare MCP tool over Wrangler …"
  },
  "cooldown_seconds": 120
}
```

v2, `distribution/rules/prefer-cf-over-wrangler.json`:

```json
{
  "on": "tool.after",
  "when": {
    "tools": ["shell", "execute"],
    "input": {
      "excludes": [
        "cf ",
        " cf",
        "\"cf\"",
        "'cf'",
        "wrangler tail",
        "wrangler secret"
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
      "id": "misuse",
      "judge": "quick",
      "ask": {
        "outcomes": ["yes", "no"],
        "evidence": ["tool_call", "request"],
        "instructions": "Does this call use the Wrangler CLI … where the cf CLI or a Cloudflare MCP tool can do it directly? … A call the user explicitly asked for, exactly as asked, does not count."
      }
    }
  ],
  "then": [
    {
      "if": {
        "misuse": "yes"
      },
      "message.append": {
        "parts": [
          {
            "text": "Chauffeur: Prefer the cf CLI or a Cloudflare MCP tool over Wrangler …"
          }
        ],
        "delivery": "now"
      }
    }
  ],
  "cooldown_seconds": 120
}
```

## `cheaper-subagent`

v1:

```json
{
  "on": "tool_result",
  "when": {
    "tools": ["subagent"],
    "input_excludes": ["\"model\""],
    "session_kind": "top_level"
  },
  "steps": [
    {
      "id": "routine",
      "question": "Is this subagent's task routine work that a smaller model handles well, …?",
      "yes_at_or_above": 0.6,
      "minimum_confidence": 0.2
    }
  ],
  "then": {
    "delivery": "steer",
    "text": "Chauffeur: the user asks that subagents doing routine work run on a cheaper model …"
  },
  "cooldown_seconds": 900
}
```

v2, `distribution/rules/cheaper-subagent.json`:

```json
{
  "on": "tool.after",
  "when": {
    "tools": ["subagent"],
    "input": {
      "excludes": ["\"model\""]
    },
    "agent": "top-level"
  },
  "judges": {
    "quick": {
      "tier": "system-one",
      "confidence": 0.6
    }
  },
  "strategy": [
    {
      "id": "routine",
      "judge": "quick",
      "ask": {
        "outcomes": ["yes", "no"],
        "evidence": ["tool_call", "request"],
        "instructions": "Is this subagent's task routine work that a smaller model handles well, …? A call the user explicitly asked for, exactly as asked, does not count."
      }
    }
  ],
  "then": [
    {
      "if": {
        "routine": "yes"
      },
      "message.append": {
        "parts": [
          {
            "text": "Chauffeur: the user asks that subagents doing routine work run on a cheaper model …"
          }
        ],
        "delivery": "now"
      }
    }
  ],
  "cooldown_seconds": 900
}
```

## `github-via-browser`

v1:

```json
{
  "on": "tool_result",
  "when": {
    "tools": ["shell", "webfetch", "execute"],
    "input_excludes": ["gh ", "git ", "\"gh\"", "\"git\""]
  },
  "steps": [
    {
      "id": "misuse",
      "question": "Does this call use a browser, browser-harness, or raw web requests … where the gh command line can do it directly? …",
      "yes_at_or_above": 0.7
    }
  ],
  "then": {
    "delivery": "steer",
    "text": "Chauffeur: Use gh for GitHub repository, PR, issue, and action access instead of browsing GitHub. …"
  },
  "cooldown_seconds": 120
}
```

v2, `distribution/rules/github-via-browser.json`:

```json
{
  "on": "tool.after",
  "when": {
    "tools": ["shell", "webfetch", "execute"],
    "input": {
      "excludes": ["gh ", "git ", "\"gh\"", "\"git\""]
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
      "id": "misuse",
      "judge": "quick",
      "ask": {
        "outcomes": ["yes", "no"],
        "evidence": ["tool_call", "request"],
        "instructions": "Does this call use a browser, browser-harness, or raw web requests … where the gh command line can do it directly? … A call the user explicitly asked for, exactly as asked, does not count."
      }
    }
  ],
  "then": [
    {
      "if": {
        "misuse": "yes"
      },
      "message.append": {
        "parts": [
          {
            "text": "Chauffeur: Use gh for GitHub repository, PR, issue, and action access instead of browsing GitHub. …"
          }
        ],
        "delivery": "now"
      }
    }
  ],
  "cooldown_seconds": 120
}
```

## Jira, Slack and X through a browser

`jira-via-browser`, `slack-via-browser` and `twitter-via-browser` differ only
in the service, the command line they exclude, and the skill they hand over:

| Rule                  | `input.excludes` | Skill         |
| --------------------- | ---------------- | ------------- |
| `jira-via-browser`    | `"jira "`        | `jira-cli`    |
| `slack-via-browser`   | `"slackcli"`     | `slack-cli`   |
| `twitter-via-browser` | `"twitter-cli"`  | `twitter-cli` |

v1, `slack-via-browser`:

```json
{
  "on": "tool_result",
  "when": {
    "tools": ["shell", "webfetch", "execute"],
    "input_excludes": ["slackcli"]
  },
  "steps": [
    {
      "id": "misuse",
      "question": "Does this call use a browser, browser-harness, or raw web requests to read or send Slack messages, …?",
      "yes_at_or_above": 0.7
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

v2, `distribution/rules/slack-via-browser.json`:

```json
{
  "on": "tool.after",
  "when": {
    "tools": ["shell", "webfetch", "execute"],
    "input": {
      "excludes": ["slackcli"]
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
      "id": "misuse",
      "judge": "quick",
      "ask": {
        "outcomes": ["yes", "no"],
        "evidence": ["tool_call", "request"],
        "instructions": "Does this call use a browser, browser-harness, or raw web requests to read or send Slack messages, …? A call the user explicitly asked for, exactly as asked, does not count."
      }
    }
  ],
  "then": [
    {
      "if": {
        "misuse": "yes"
      },
      "message.append": {
        "parts": [
          {
            "text": "Chauffeur: Use slackcli for Slack instead of the browser; the skill below shows how."
          },
          {
            "skill": "slack-cli"
          }
        ],
        "delivery": "now"
      }
    }
  ],
  "cooldown_seconds": 120
}
```

The `skill` part attaches the skill's body to the same message. The three
skills ship in `distribution/skills/handoff/`, which the
[skills plugin](skills.md) serves.
