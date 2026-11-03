# Context renewal and warming

v1 kept an idle session's prompt cache warm, and when the user came back after
it had expired to a long conversation, had a cheaper model summarize it before
the prompt ran on the user's own model. In v2 these are the
`context-renewal` and `warming` plugins, in slice 7. Their code is in the
plan's [Plugins from v1's features](../plans/pluggability.md#plugins-from-v1-s-features).

## Context renewal

| v1 | v2 |
|---|---|
| `context_resumed` before a prompt that continues a measured context | `session.resumed` |
| The facts judge: renew when idle at least the cache lifetime and the context at least the budget | Plain code in the plugin: no question is asked |
| `cache_seconds` per provider: Anthropic 300, OpenAI 1800, DeepSeek 3600, others 300 | The same option |
| Budget: 0.215 of the window, between 50,000 and 100,000 tokens | The same option |
| Summary model: `renewal.pins`, a fast-tier model on the agent's provider, another provider's, then a free one; never blacklisted or limited | The same order, from the model router's provider tables |
| Fast-tier and last-resort sessions left to OpenCode's own compaction | The same |
| The summary written in a temporary child session on the summary model, then removed | `session.create`, `session.generate`, `session.remove` through the harness package |
| A failed summary passes over its provider for an hour | The same, in the plugin's state |
| OpenCode's template from `hosts/opencode.json` | The harness package's `compaction.before` supplies it |

## Warming

| v1 | v2 |
|---|---|
| `context_idle` after a turn the user drives | `session.idle` |
| A keep-alive every `cache_seconds − 60`, within an hour, while each costs less than re-sending | The same schedule, from the plugin's options and the event's prices |
| Not warmed: under 20,000 tokens, an unknown cache-read price, a limited provider, a subagent | The same |
| `session.generate` with the keep-alive prompt | `session.warm`; where the harness only has its own warming setting, the plugin does nothing |

## Options

```jsonc
{
  "context-renewal": {
    "options": {
      "cache_seconds": { "anthropic": 300, "openai": 1800, "deepseek": 3600, "*": 300 },
      "budget": { "share": 0.215, "min_tokens": 50000, "max_tokens": 100000 },
      "pins": []
    }
  },
  "warming": {
    "options": { "max_seconds": 3600, "lead_seconds": 60, "min_tokens": 20000 }
  }
}
```
