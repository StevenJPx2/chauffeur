# OpenCode adapter

An OpenCode plugin that sends each completed tool call to the Chauffeur
daemon as `tool.after` and carries out what the rules return. It decides
nothing. See [slice 1](../../docs/plans/slices/slice-1.md#the-opencode-adapter).

| File            | Does                                                          |
| --------------- | ------------------------------------------------------------- |
| `src/index.ts`  | The plugin: hooks `execute.after`, logs trouble once a minute |
| `src/event.ts`  | A completed call as a `tool.after` event                      |
| `src/daemon.ts` | The installation, its socket, and one `chauffeur.emit` call   |
| `src/act.ts`    | `message.append` with `delivery: "now"` onto the result       |
| `test/`         | `bun test` against a stand-in daemon on a real socket         |

```sh
bun install
bun run check   # typecheck, test, and build dist/server.js
```

The release copies `dist/server.js` into `share/<name>/opencode/`, where
OpenCode loads it as a local plugin directory.
