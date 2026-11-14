# Context renewal and warming

v1 kept an idle session's prompt cache warm, and when the user came back after
it had expired to a long conversation, had a cheaper model summarize it before
the prompt ran on the user's own model. In v2 these are the
[`context-renewal`](../plugins/context-renewal.md) and
[`warming`](../plugins/warming.md) plugins, in slice 7.

## Context renewal

| v1                                                                                                                                                                  | v2                                                                                                                                |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `context_resumed` before a prompt that continues a measured context                                                                                                 | `session.resumed`                                                                                                                 |
| The facts judge: renew when idle at least the cache lifetime and the context at least the budget                                                                    | Plain code in the plugin: no question is asked                                                                                    |
| The model's `cache.ttl` as the user set it in OpenCode, a variant's first; else `cache_seconds` per provider: Anthropic 300, OpenAI 1800, DeepSeek 3600, others 300 | The adapter sends the model's `cache_seconds` on `session.resumed` and `session.idle` when the user set one; else the same option |
| Budget: 0.215 of the window, between 50,000 and 100,000 tokens                                                                                                      | The same option                                                                                                                   |
| Summary model: `renewal.pins`, a fast-tier model on the agent's provider, another provider's, then a free one; never blacklisted or limited                         | The same order, from the model router's provider tables                                                                           |
| Fast-tier and last-resort sessions left to OpenCode's own compaction                                                                                                | The same                                                                                                                          |
| The summary written in a temporary child session on the summary model, then removed                                                                                 | `session.create`, `session.generate`, `session.remove` through the adapter                                                        |
| A failed summary passes over its provider for an hour                                                                                                               | The same, in the plugin's state                                                                                                   |
| OpenCode's template from `hosts/opencode.json`                                                                                                                      | The adapter's `compaction.before` supplies it                                                                                     |
| The summary model writes only a compaction that starts within 2 minutes; a later one is OpenCode's own                                                              | The same window, in `compaction.before`                                                                                           |
| `renew_when_idle`: a context at its budget renewed while idle, after the cache lifetime, the warming span and 60 s, unless the session runs first                   | `session.idle` in the plugin, with the same delay                                                                                 |
| `awaiting_answer`: a turn waiting on the agent's question renewed after the cache lifetime and 60 s; the turn is interrupted and the agent asked to ask again       | The harness's question event; `session.interrupt`, `session.compact`, then `message.append`                                       |

## Warming

| v1                                                                                                                                            | v2                                                                                          |
| --------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| `context_idle` after a turn the user drives                                                                                                   | `session.idle`                                                                              |
| A keep-alive every `cache_seconds − 60`, within an hour, while each costs less than re-sending; once, at 59 minutes, for a 1-hour `cache.ttl` | The same schedule, from the plugin's options and the event's `cache_seconds` and prices     |
| Not warmed: under 20,000 tokens, an unknown cache-read price, a limited provider, a subagent, a turn waiting on a question                    | The same                                                                                    |
| A warming request that writes more of the cache than it reads stops the session's warming                                                     | The same                                                                                    |
| `session.generate` with the keep-alive prompt                                                                                                 | `session.warm`; where the harness only has its own warming setting, the plugin does nothing |

`model-router.json`'s `renewal` settings become the two plugins'
[renewal](../plugins/context-renewal.md#options) and
[warming](../plugins/warming.md#options) options.
