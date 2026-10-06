# Prompt trimming

Cut the fixed part of every OpenCode request without losing task outcomes. Each
trim ships only when `chauffeur-bench` shows it saves tokens at the same pass
rate.

## Where the tokens go

A fresh session's first request on `anthropic/claude-opus-5-5` (OpenCode
2.0.23, this machine's global config) is 13,442 tokens. By share of the request
body:

| Part | Characters | Share | Owner |
| --- | ---: | ---: | --- |
| Direct tool definitions (19 tools) | 16,683 | 46% | OpenCode, plugins |
| Code Mode catalog and MCP guidance | 10,436 | 29% | OpenCode, MCP servers |
| sourcefed monitoring guidance | 4,117 | 11% | sourcefed plugin |
| OpenCode base instructions | 1,773 | 5% | OpenCode |
| OptMem memory instructions | 1,632 | 5% | OptMem plugin |
| Model name, worktree note, user message | 453 | 1% | OpenCode |

The longest tool definitions are `subagent` (3,000), `shell` (1,484),
`question` (1,415), `execute` (1,173), `edit` (1,143), `grep` (1,078) and
`todowrite` (1,001); the six `memo_*` tools add 2,424.

In an isolated bench run (`chauffeur-bench prompt`, no OptMem or sourcefed,
MCP servers settled), the first request of a one-word task is:

| Model | Without Chauffeur | With Chauffeur |
| --- | ---: | ---: |
| `anthropic/claude-opus-5-5` | 18,758 tokens | 10,437 tokens (−44%) |
| `openai/gpt-6-luna` | 11,293 tokens | 5,882 tokens (−48%) |

Most of the difference is OpenCode's skill list (`<available_skills>`, 24,000
characters), which Chauffeur removes. Chauffeur adds `todowrite` and
`ask_chauffeur` (1,700 characters) and the cheaper-subagent note on
`subagent`. The Code Mode catalog (15 namespaces, 7,900 characters plus MCP
guidance) is the same in both.

Over a session the history dominates: requests in the last seven days averaged
396k tokens, 95.6% of them cache reads, so the fixed part is about 3% of an
average request. Trimming pays most in short sessions, subagents, and the first
request after each compaction.

## Scenarios

Each scenario is a pair of bench variants that differ in one thing.

| Scenario | Control | Treatment | Wins when |
| --- | --- | --- | --- |
| `codemode-trim` | `full` | `full` + Code Mode namespaces withheld until judged needed | pass rate rises or holds with fewer steps on the namespace tasks; the needed namespace is listed before first use in ≥ 90% of runs |
| `short-tools` | `full` | `full` + shortened descriptions for the longest direct tools | first-request tokens fall; pass rate holds; subagent and question use stays within noise |
| `sourcefed-guidance` | `full` + sourcefed | `full` + sourcefed with short guidance | first-request tokens fall; monitor tasks still create the right monitor |
| `memo-wake` | `full` + OptMem | `full` + OptMem with a capped wake | second-request tokens fall; memory tasks still recall the fixture fact |

"Pass rate holds" means no task loses a pass across three repeats, and the
suite's pass rate is within one run of the control.

## Harness

`chauffeur-bench` gains four things.

**Isolation.** A run sees only what its variant names. The run's plugin list
disables every optional global plugin (`-ntfy-notify`, `-sourcefed`, `-optmem`)
unless the variant enables it, and the daemon reads `CHAUFFEUR_CONFIG_DIR` set to
the run's own config folder, so personal rules and overrides stay out.

**Variant config and plugins.** A variant may name a config overlay folder and
plugins to keep:

```json
{ "id": "short-tools", "chauffeur": true, "disable": [], "config": "variants/short-tools" }
{ "id": "sourcefed", "chauffeur": true, "disable": [], "plugins": ["sourcefed"] }
```

`config` is copied into the run's config folder (`hosts/opencode.json`,
`tool-exposure.json`, …), so a scenario is a few JSON overrides.

**Prompt measurement.** `chauffeur-bench prompt --variants full,short-tools`
runs one fixed prompt per variant with a probe plugin in the run's
`.opencode/plugins/`. The probe records the outgoing request body by section
(system parts, each tool definition, messages) from the `http.request` hook, and
the result adds the provider's token counts for the first two requests. The
report is a per-variant table like the one above, with the difference against
the control. It costs one short run per variant and per model, so it runs on
`openai/gpt-6-luna` and `anthropic/claude-opus-5-5`, which assemble different
requests.

**Task MCP servers.** A task may ship stub MCP servers in `mcp/`, registered in
the run's `.opencode/opencode.jsonc` under `mcp.servers`, so a task can need a
Code Mode namespace. Stubs log their calls to `$BENCH_LOG` like `bin/` stubs.
New tasks:

- `tracker-create-issue`: file an issue through a stub `tracker` MCP server; the
  check reads the logged call.
- `docs-lookup-answer`: answer from a stub `handbook` MCP server's search tool.
- `memory-recall` (for `memo-wake`): recall a fact from a fixture OptMem store.

**New result fields**, from OpenCode events and the audit log: first-request and
second-request prompt tokens, Code Mode namespaces revealed and when,
`ask_chauffeur` calls, missing-tool recoveries, subagent launches, and
`question` calls.

## Code Mode trimming

Code Mode namespaces are withheld like a skill is left unattached: judged at a
context's first message, restored when a request needs them.

1. **Withheld at a first message.** Tool exposure already asks, per namespace,
   whether the request needs it. With `tool-exposure.json` `code_mode.trim` on,
   the same answer at P ≤ 0.3 withholds the namespace, except
   `code_mode.always` (`context7`, `jina`, `opencode`). The `namespaces` effect
   names them; the adapter adds session deny rules (`<namespace>_*`, or one per
   permission action when that pattern would catch other tools), keeping the
   session's other rules. OpenCode leaves wholly denied tools out of the
   catalog, `search`, and MCP guidance. A first message also covers a context
   after compaction, when OpenCode rebuilds the baseline.
2. **Restored when needed.** A later message judged to need a withheld
   namespace, or an `ask_chauffeur` grant, restores it by removing exactly
   those rules; a short "new namespaces" note joins the next request. Nothing
   is withheld mid-context, since a removal note would add tokens while the
   baseline keeps the full catalog.
3. **Complete catalog.** At a first message the adapter waits (at most 10 s)
   until no MCP server is connecting, so the judgment covers the namespaces
   OpenCode's first request will carry.

**Measured:** withholding 11 of 15 namespaces leaves the first request the same
size (5,878 vs 5,880 tokens on Luna). OpenCode 2.0.23's catalog lists a fixed
number of tools (about 28 of 340), so the remaining namespaces fill the space.
Trimming saves tokens only if OpenCode ties the listing to the catalog's size;
its remaining value is a catalog focused on the task, which the outcome suite
measures. `code_mode.trim` stays off unless that helps.

## Order of work

1. Harness: isolation, variant `config` and `plugins`, `prompt` command, new
   result fields. Record the control's numbers on both models.
2. `short-tools`: description overrides through `hosts/opencode.json`, applied
   with `tool.transform` updates; bench.
3. `codemode-trim`: MCP stub tasks, then the feature; bench.
4. `sourcefed-guidance` and `memo-wake`: changes in those plugins' repos; bench.
5. README: state what Chauffeur trims, with the measured numbers.

## Open questions

- sourcefed's guidance: whether a short always-on paragraph plus the full text
  through `sourcefed skills get core` keeps monitor tasks passing.
- OptMem: a fixture store for bench runs (a memory folder chosen by environment
  variable), so `memo-wake` never reads the real memory.
