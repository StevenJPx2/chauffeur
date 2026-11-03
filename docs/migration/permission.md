# Permissions

v1's permission capability had three parts: task-approval contracts that
answer prompts the user would have approved, shell guards that deny scripted
reads and writes of project files, and a safety backstop. In v2 the contracts
and guards are rule files on `permission.asked`, and the backstop is the
`guard` plugin. All of it lands in slice 5.

| v1                                                         | v2                                                              | Kind   |
| ---------------------------------------------------------- | --------------------------------------------------------------- | ------ |
| `workspace-edit-gate`                                      | `distribution/rules/permission/edit.json`                       | Rule   |
| `workspace-shell-gate`                                     | `distribution/rules/permission/shell.json`                      | Rule   |
| `external-directory-gate`                                  | `distribution/rules/permission/outside-directory.json`          | Rule   |
| `shell-file-write-guard`                                   | `distribution/rules/permission/shell-writes.json`               | Rule   |
| `shell-file-read-guard`                                    | `distribution/rules/permission/shell-reads.json`                | Rule   |
| `safety/backstop.json` patterns and `confirm`              | `distribution/safety/backstop.json`, read by the `guard` plugin | Plugin |
| The irreversible-harm question and `learned-backstop.json` | The `guard` plugin, with its learned list in its own state      | Plugin |

## How a permission is answered

- The adapter sends every request the harness does not deny, with the
  harness's own decision as `host`: `ask` or `allow`.
- Every rule whose `when` matches answers in the same round. Its `then`
  returns `permission.reply` with `allow`, `ask` or `deny`.
- The most restrictive reply wins: `deny`, then `ask`, then `allow`. A
  harness denial always stands, and a request no rule answers keeps the
  harness's decision.
- Within the 5-second budget, a failure or a late answer keeps the
  harness's decision: Chauffeur only vetoes what the harness allows, and an
  `ask` still asks.
- A resource over 16 KiB, or resources over 128 KiB together, is never
  clipped: the request is asked.

## `when` on `permission.asked`

| v1                                                    | v2                                                       |
| ----------------------------------------------------- | -------------------------------------------------------- |
| `match.actions`                                       | `when.actions`: `edit`, `shell`, `external_directory`, … |
| `match.host_decisions` (default `ask`)                | `when.host` (default `["ask"]`)                          |
| `required_evidence: ["resource_present"]`             | `"resources": { "present": true }`                       |
| `resource_within_workspace`, `resource_named_by_user` | `resources.within_workspace`, `resources.named_by_user`  |

## A task-approval gate

v1, `skills/permission/workspace-edit-gate.json`, abridged:

```json
{
  "match": {
    "events": ["permission.evaluate"],
    "actions": ["edit"]
  },
  "predicates": {
    "required_evidence": ["resource_present"]
  },
  "decision": {
    "id": "edit_serves_the_task",
    "type": "noul",
    "instructions": "OpenCode would ask the user before this edit. …",
    "positive_at_or_above": 0.7,
    "negative_at_or_below": 0.3,
    "minimum_confidence": 0.4
  },
  "outcomes": {
    "positive": {
      "effect": "allow",
      "message": "Chauffeur approved this edit: …"
    },
    "negative": {
      "effect": "ask",
      "message": "…"
    },
    "uncertain": {
      "effect": "ask",
      "message": "…"
    },
    "missing_evidence": {
      "effect": "ask",
      "message": "The edit names no file."
    },
    "judge_failure": {
      "effect": "ask",
      "message": "…"
    }
  }
}
```

v2, `distribution/rules/permission/edit.json`:

```json
{
  "on": "permission.asked",
  "when": {
    "actions": ["edit"],
    "host": ["ask"],
    "resources": {
      "present": true
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
      "id": "serves",
      "judge": "quick",
      "ask": {
        "outcomes": ["yes", "no"],
        "evidence": ["permission", "request", "activity"],
        "instructions": "OpenCode would ask the user before this edit. Is changing this exact file clearly part of carrying out the user's stated task? …"
      }
    }
  ],
  "then": [
    {
      "if": {
        "serves": "yes"
      },
      "permission.reply": {
        "decision": "allow",
        "message": "Chauffeur approved this edit: it serves your stated task."
      }
    },
    {
      "if": {
        "serves": "no"
      },
      "permission.reply": {
        "decision": "ask",
        "message": "Chauffeur did not approve this edit; it looks outside your stated task."
      }
    },
    {
      "if": {
        "serves": "unsure"
      },
      "permission.reply": {
        "decision": "ask",
        "message": "Chauffeur is unsure this edit serves your task; approve it if you intended it."
      }
    },
    {
      "if": {
        "serves": "failed"
      },
      "permission.reply": {
        "decision": "ask",
        "message": "Chauffeur could not judge this edit; approve it if you intended it."
      }
    }
  ]
}
```

`missing_evidence` needs no entry: without a resource the rule does not
match, and the harness asks. The shell and outside-directory gates differ
only in `actions`, the instructions and the messages.

## A shell guard

Guards also judge commands the harness allows, and only a confident `yes`
does anything. v2, `distribution/rules/permission/shell-writes.json`:

```json
{
  "on": "permission.asked",
  "when": {
    "actions": ["shell"],
    "host": ["ask", "allow"],
    "resources": {
      "present": true
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
      "id": "writes",
      "judge": "quick",
      "ask": {
        "outcomes": ["yes", "no"],
        "evidence": ["permission"],
        "instructions": "Does this shell command write or change a project file (source, config, tests or docs) by script, …?"
      }
    }
  ],
  "then": [
    {
      "if": {
        "writes": "yes"
      },
      "permission.reply": {
        "decision": "deny",
        "message": "Chauffeur: change project files with the edit, write, or patch tools, …"
      }
    }
  ]
}
```

v1's `abstain` outcomes are the entries a guard leaves out. The read guard
is the same with the read question and message. `cat-to-read` in
[steering](steering-rules.md#cat-to-read) remains the reminder after a read
that got through.

## The backstop

The `guard` plugin hooks `tool.before` and `permission.asked`:

| Check                                                                                      | Runs                                                          | Then                                                                                                     |
| ------------------------------------------------------------------------------------------ | ------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| `patterns` from `backstop.json`, shipped and the user's (`replace` drops the shipped list) | Before any model call                                         | Block                                                                                                    |
| `confirm` patterns, such as a force-push to `main`                                         | Before any model call                                         | Ask the user                                                                                             |
| The learned list                                                                           | Before any model call                                         | Block                                                                                                    |
| "Could this command cause irreversible harm?" on every shell command                       | One System One step, `confidence: { "yes": 0.9, "no": 0.95 }` | `yes` blocks and adds the command to the learned list; `unsure`, `failed` or a slow answer asks the user |

A pattern that names a root matches only the root, so `rm -rf /tmp/build`
passes. The learned list lives in the plugin's state, apart from
configuration, so it can be reviewed. The plugin's code is in the plan's
[Plugins from v1's features](../plans/pluggability.md#plugins-from-v1-s-features).
