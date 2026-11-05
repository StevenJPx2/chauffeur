# Tools

Tools are what Chauffeur gives the agent to call. A plugin registers each one
with `tool.add`, and the harness's adapter puts it in the harness's tool
list, beside the harness's own tools.

| Tool                                | Does                                                         | From                                                      | Slice |
| ----------------------------------- | ------------------------------------------------------------ | --------------------------------------------------------- | ----- |
| [`ask_chauffeur`](ask-chauffeur.md) | The agent asks, in plain words, for a tool or skill it lacks | The [`ask-chauffeur`](../plugins/ask-chauffeur.md) plugin | 6     |

## How a call looks

The agent calls a plugin's tool as it calls any other, with the tool's
input as JSON:

```json
{ "tool": "ask_chauffeur", "input": { "need": "post a reply in a Slack thread" } }
```

The reply is text the agent reads in the same step. `ask_chauffeur` is never
hidden and never moved into Code Mode, so the agent can always reach it.

To add a tool, see [Writing a plugin](../plugins/writing.md#tools).
