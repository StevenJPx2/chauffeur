# `chauffeur navigate`

Sends one `chauffeur.navigate` request, read as JSON on stdin, as a caller
does, and prints each JSON-RPC message that comes back, one per line: the
run's events as they happen, then the intents or, for an inline strategy,
the verdict. For hook scripts, Git hooks and CI on the same machine.

```sh
chauffeur navigate <<'EOF'
{ "protocol": 1, "deadline_ms": 3000, "on": "sourcefed.event",
  "evidence": { "event": "thanks, looks good 👍", "monitor": "PR #42" } }
EOF
```

```text
{"jsonrpc":"2.0","method":"chauffeur.event","params":{"event":"rule_matched","id":1,"rule":"sourcefed/triage"}}
{"jsonrpc":"2.0","method":"chauffeur.event","params":{"event":"round_ready","id":1,"round":1,"rule":"sourcefed/triage","steps":["screen"]}}
{"jsonrpc":"2.0","method":"chauffeur.event","params":{"answer":{"level":0.9,"model":"jev","outcome":"no",…},"event":"step_settled","id":1,"rule":"sourcefed/triage","step":"screen"}}
{"id":1,"jsonrpc":"2.0","result":{"intents":[{"rule":"sourcefed/triage","withhold":"event"}]}}
```

| Exit | Means                                                                        |
| ---- | ---------------------------------------------------------------------------- |
| 0    | Intents or a verdict came back                                               |
| 1    | The request was rejected, such as `limit_exceeded`, or stdin is not JSON     |
| 2    | No daemon answers, it does not answer in time, or it speaks another protocol |

A script tests the reply with `jq`:

```sh
if chauffeur navigate < request.json | tail -1 \
  | jq -e '.result.intents | any(.withhold == "event")' > /dev/null; then
  echo "withheld"
fi
```

It waits for the request's `deadline_ms`, 3 seconds by default, and 2
seconds more. The socket is `$CHAUFFEUR_SOCKET`, or the installation's own.
The wire contract is in [Slice 2](../plans/slices/slice-2.md#wire-contract).
