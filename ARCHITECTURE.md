# Chauffeur architecture

Chauffeur loads versioned JSON skill contracts and enforces them at an agent
host boundary. A contract matches a host event and action, checks deterministic
evidence, requests a typed local judgment when eligible, and returns an effect
with explicit failure outcomes. Idle reminders remain available as an optional
runtime feature.

## Package boundaries

```text
skills/*.json ─────────────┐
plugins/* ─────────────────┤
judges/* ──────────────────┤
                           ▼
                    chauffeur-core
                    (Rust SDK)
                           │
                           ▼
                    chauffeur-daemon
                      HTTP RPC + SSE
                     /      |      \
                    ▼       ▼       ▼
             chauffeur-cli  MCP  adapters/*
                                  agent hosts
```

| Path | Responsibility |
|---|---|
| `crates/core` | Skill contract types, strict validation, deterministic evaluation, judge interface, optional idle steering, reminder queue, RPC contracts and client |
| `crates/daemon` | Loads skill JSON at startup, composes provider plugins and Laya, and exposes the core runtime over HTTP RPC and SSE |
| `crates/cli` | Starts the daemon and provides health, skill validate/evaluate/list, steering, and reminder commands |
| `crates/mcp` | Exposes daemon-backed tools over MCP stdio |
| `plugins/github` | Optional GitHub rules for idle steering |
| `plugins/jira` | Optional Jira rules for idle steering |
| `plugins/git` | Optional Git rules for idle steering |
| `judges/laya` | Native JSONL Unix-socket client for the installed Laya Core ML/ANE daemon |
| `adapters/opencode` | OpenCode V2 plugin; enforces skill effects at permission evaluation and retains idle reminder delivery |

`chauffeur-core` owns policy and evaluation. The daemon, CLI, MCP server, and
host adapters are transport boundaries. Plugin and judge crates are assembled
by the daemon.

## Skill contract

Each `skills/*.json` file is a complete contract with these sections:

- `schema_version`, `identity.id`, `identity.name`, and `identity.version`
- `match.events` and `match.actions` for exact host event and action matching
- `predicates.required_evidence` and `predicates.forbidden_evidence` for
  deterministic Boolean evidence keys
- `decision` for a `choice`, `score`, or `noul` Laya question, confidence floor,
  explicit numeric thresholds or choice mapping, and bounded criteria
- `outcomes.positive`, `negative`, `uncertain`, `missing_evidence`,
  `judge_failure`, and `cooldown`
- `cooldown_seconds` for per-agent, per-skill evaluation throttling

The loader accepts schema version 1, rejects unknown fields, and validates every
contract before daemon activation. Skill files are limited to 65,536 bytes and
the directory may contain up to 64 JSON files and 256 entries total. The loader
rejects duplicate skill IDs. Match lists must be non-empty. The CLI uses the
same loader as the daemon.

At evaluation, Chauffeur checks event/action matching and all evidence
predicates before calling Laya. Required evidence must be present and `true`;
forbidden evidence must be absent or `false`. A failed predicate selects the
deterministic `missing_evidence` outcome and never reaches the judge. Laya
confidence below `minimum_confidence` selects `uncertain`. Numeric judgments
must satisfy the skill's declared bounds and thresholds; choice judgments must
match one of the declared criterion values. Invalid answers and judge errors
select `judge_failure`.

Skill contexts are limited to 65,536 encoded bytes, with a 16,384-byte state,
32 evidence entries, 120-byte event/action/evidence-key fields, and a 128-byte
agent ID. Skill text, criteria, and cooldowns have individual limits. Laya
requests and responses are each limited to 65,536 bytes. The daemon caps an RPC
request body at 128 KiB before JSON decoding.

The sample `skills/workspace-edit-gate.json` matches OpenCode `edit` permission
evaluations. The adapter supplies three deterministic evidence keys:

- `resource_present`: the request names at least one resource and stays within
  the resource count and size limits
- `resource_within_workspace`: every resource resolves inside the canonical
  workspace, including symlink resolution for existing paths
- `resource_named_by_user`: the exact resource path or its filename appears in
  a recent user message

The skill asks for user review when any key is false. Laya can choose an
allow/deny branch only after all three deterministic requirements pass.

## Evaluation flow

```text
OpenCode permission.evaluate
          │
          ▼
adapter builds bounded SkillContext + Boolean evidence
          │
          ▼
POST /rpc: skill.evaluate(skill_id, context)
          │
          ├─ event/action mismatch ───────────────► no effect
          ├─ missing/forbidden evidence ──────────► deterministic outcome
          ├─ cooldown active ────────────────────► cooldown outcome
          └─ eligible ─► Laya typed judgment ────► thresholded outcome
                                                     │
                                                     ▼
                                    permission effect + user-facing message
```

The OpenCode adapter combines matching skill outcomes by restriction:
`deny` takes precedence over `ask`, which takes precedence over `allow`. An
initial host denial remains in force. An `allow` outcome takes effect only
after the contract's deterministic evidence checks pass and its Laya threshold
is met. Uncertain, missing-evidence, and judge-failure outcomes cannot allow.
Cooldown outcomes cannot allow either, because they return without consulting
Laya.
`prompt` and `remind` outcomes become permission asks with their reminder text.
When contract evaluation or the daemon fails, the adapter asks for user review.

Cooldown history is held in daemon memory, keyed by agent and skill ID, and
bounded to 4,096 entries. A daemon restart clears that history. If the bound is
full, Chauffeur fails the evaluation so the adapter asks for review.

## Laya boundary

The daemon connects to the user-level Laya Unix socket and checks health before
serving. The default is
`~/Library/Application Support/laya/laya.sock`; `LAYA_SOCKET` overrides it.
Prediction requests use the native JSONL protocol:

```json
{"state":"...","questions":{"edit_is_in_user_scope":{"type":"choice","instructions":"...","criteria":["within_scope","outside_scope"]}}}
```

Laya returns typed `choice`, `score`, or `noul` values with confidence. It never
returns effect decisions or reminder prose. Chauffeur validates the value,
checks confidence and the contract's hard predicates, then selects the static
outcome stored in the JSON file.

## Daemon protocol

- `POST /rpc`: JSON `{ "id?", "method", "params" }` request and
  `{ "id?", "result?", "error?" }` response
- `GET /events?target=<json>`: target-scoped SSE for queued idle reminders
- Optional bearer authentication through `CHAUFFEUR_DAEMON_TOKEN`
- Default address `127.0.0.1:18788`

| Method | Purpose |
|---|---|
| `health` | Check daemon readiness |
| `skills.list` | List validated skill IDs |
| `skill.evaluate` | Evaluate one skill against a `SkillContext` |
| `steer` | Evaluate optional idle rules and queue reminders |
| `reminders.read` | Read unacknowledged reminders for a target |
| `reminders.acknowledge` | Remove reminders after host delivery |

The reminder queue is bounded to 50 entries per target and persists atomically.
SSE replays unacknowledged reminders after reconnect and sends a heartbeat every
15 seconds.

## Optional idle steering

Provider plugins contribute deterministic gates and static reminder templates.
The judge evaluates only fuzzy applicability and urgency. The core blocks
ineligible contexts before Laya, caps each idle result at two reminders, and
records cooldown only after successful queue delivery. The adapter observes
idle status, keeps a bounded history of 40 tool calls, prompts the session with
queued reminders, and acknowledges each reminder after delivery. OpenCode idle
hooks are disabled by default and activate when
`CHAUFFEUR_IDLE_STEERING=true`.

## Configuration

| Variable | Default |
|---|---|
| `CHAUFFEUR_DAEMON_URL` | `http://127.0.0.1:18788` |
| `CHAUFFEUR_DAEMON_TOKEN` | unset |
| `CHAUFFEUR_STATE_DIR` | `~/.local/state/chauffeur` |
| `CHAUFFEUR_SKILLS_DIR` | `skills` directory three levels above the executable; otherwise `./skills` |
| `CHAUFFEUR_IDLE_STEERING` | `false` |
| `CHAUFFEUR_BIN` | `chauffeur` |
| `LAYA_SOCKET` | `~/Library/Application Support/laya/laya.sock` |
