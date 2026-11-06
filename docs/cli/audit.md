# `chauffeur audit`

Prints the runs in the audit log, oldest first. It reads the log's files, so
it works with no daemon running, and shows runs from before a restart.

```sh
chauffeur audit                    # the last 20 runs, one JSON line each
chauffeur audit 100
chauffeur audit --follow --brief   # new runs as they happen, one short line each
```

| Argument or option | Does                                                                       |
| ------------------ | -------------------------------------------------------------------------- |
| `N`                | How many of the most recent runs to print; `20` by default, `0` for none   |
| `--follow`         | Then keeps printing each run as it is written, missing none, until stopped |
| `--brief`          | One short line per run in place of its JSON                                |

A run's JSON line:

```json
{
  "at": "2026-11-03T13:54:43.412075Z",
  "elapsed_ms": 312,
  "event": "tool.after",
  "returned": "message.append",
  "rule": "conventional-commits",
  "session": "s1",
  "steps": [
    {
      "step": "nonconventional",
      "answer": {
        "outcome": "yes",
        "p": { "yes": 0.93, "no": 0.07 },
        "level": 0.07,
        "model": "jev",
        "placeholder": false
      }
    }
  ]
}
```

The same run with `--brief`, in local time:

```text
13:54:43  conventional-commits  s1  nonconventional yes 0.93 jev  312 ms  → message.append
```

| A run that                                                        | Has                                                        |
| ----------------------------------------------------------------- | ---------------------------------------------------------- |
| Answered                                                          | `steps`: each step's `answer`, or `failed` with the reason |
| Was cooling down                                                  | `"cooldown": true`, and no steps                           |
| Could not run, such as a placeholder whose answers would not load | `refused` with the reason                                  |
| Missed its deadline                                               | `"deadline": true`                                         |
| Returned something                                                | `returned`: the action or intent                           |

The log is `audit.jsonl` in `~/.local/state/<name>/`, a folder only you can
open. Past 8 MiB it moves to `audit.1.jsonl`, replacing the one there, so
the log keeps at most about 16 MiB; `audit` reads across both files.
