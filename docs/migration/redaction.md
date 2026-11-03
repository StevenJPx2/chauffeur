# Redaction

Redaction runs in the engine before any model call, as in v1. It ships in
slice 1, with v1's rules and test cases.

| v1                                                                                             | v2                                                                                                                          |
| ---------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| Shipped prefixes in `skills/safety/redaction.json`                                             | `distribution/safety/redaction.json`                                                                                        |
| `$CHAUFFEUR_CONFIG_DIR/redaction.json` adds, or replaces with `"replace": true`                | `~/.config/chauffeur/redaction.json`, the same                                                                              |
| Private-key blocks become `[REDACTED]`                                                         | The same                                                                                                                    |
| Credential-like strings (24+ mixed, or 32+ hex after a key) masked by shape                    | The same, numbered per navigation                                                                                           |
| A bare hash passes; paths and URLs are judged one segment at a time                            | The same                                                                                                                    |
| Learned shapes: Jev asks whether a masked shape is a secret, saved to `learned-redaction.json` | Slice 5, with the guard's learned list: the same question at `yes` 0.7, kept in Chauffeur's state, apart from configuration |
| Audit fields use the same redactor                                                             | Events and logs use the same redactor                                                                                       |

The rules, the matching tests and the module layout are in
[`crates/runtime/src/redact/`](../../crates/runtime/src/redact/).
