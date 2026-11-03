# `chauffeur navigate`

Arrives with slice 2. Sends one `navigate` request, read as JSON on stdin, as
a caller does, and prints each JSON-RPC message that comes back: the run's
events as they happen, then the intents or, for an inline strategy, the
verdict. For hook scripts, Git hooks and CI on the same machine.

```sh
chauffeur navigate <<'EOF'
{ "protocol": 1, "deadline_ms": 3000, "on": "sourcefed.event", "evidence": { "event": "…", "thread": "…" } }
EOF
```

| Exit | Means                                                            |
| ---- | ---------------------------------------------------------------- |
| 0    | Intents or a verdict came back                                   |
| 1    | The request was rejected, such as `limit_exceeded` or `deadline` |
| 2    | No daemon answers                                                |

The wire contract is in the plan's [Slice 2](../plans/slices/slice-2.md#wire-contract).
