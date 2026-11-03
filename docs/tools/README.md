# Tools

Tools are what Chauffeur gives the agent to call:

| Tool                                | Does                                                               | From                                                      | Through     | Slice |
| ----------------------------------- | ------------------------------------------------------------------ | --------------------------------------------------------- | ----------- | ----- |
| [`ask_chauffeur`](ask-chauffeur.md) | The agent asks, in plain words, for a tool or skill it lacks       | The [`ask-chauffeur`](../plugins/ask-chauffeur.md) plugin | The adapter | 6     |
| [`judge`](judge.md)                 | The agent asks Chauffeur's judges a question about evidence it has | Chauffeur                                                 | MCP         | 6     |

They reach an agent two ways:

| Way                                  | For                                                    | Serves                                                    |
| ------------------------------------ | ------------------------------------------------------ | --------------------------------------------------------- |
| The harness package's adapter        | A harness Chauffeur rides along with, such as OpenCode | Every tool a plugin registers with `tool.add`             |
| The [`chauffeur` MCP server](mcp.md) | Any MCP client, with or without an adapter             | `judge`, and the plugin tools that work outside a session |

Use one way per harness for a given tool: the adapter already registers the
plugins' tools, so adding the MCP server there only adds `judge`.

## How a call looks

A tool is called directly, or through Code Mode where the harness has it:

```json
{ "tool": "ask_chauffeur", "input": { "need": "post a reply in a Slack thread" } }
```

```ts
// OpenCode, Code Mode: MCP tools sit under their server's namespace
const answer = await tools.chauffeur.judge({
  question: "Does this diff change the public API?",
  evidence: { diff }
})
```

`ask_chauffeur` is never in Code Mode, so the agent can always reach it.
