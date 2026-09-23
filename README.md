# Chauffeur

Chauffeur turns versioned JSON skill contracts into decisions at coding-agent
boundaries. Contracts match host events and actions, check deterministic
evidence, optionally ask the local Laya model a typed question, and select an
explicit effect. Idle reminders remain available as an optional feature.

See [`ARCHITECTURE.md`](./ARCHITECTURE.md) for package boundaries, the skill
evaluation flow, limits, and failure behavior.

## Prerequisites

Install the persistent, user-level Laya daemon once:

```sh
cd /path/to/laya
tools/install-daemon.sh
```

The installer registers `com.laya.daemon` with launchd and reports readiness at
`~/Library/Application Support/laya/laya.sock`. Chauffeur uses this native
JSONL Unix-socket API directly.

## Build and validate

```sh
cargo build --release
./target/release/chauffeur skill validate skills/workspace-edit-gate.json
```

Validation uses the same strict JSON schema and size limits as daemon startup.
The loader rejects unknown fields, invalid thresholds, duplicate IDs, and
unsupported schema versions.

Set `CHAUFFEUR_SKILLS_DIR` to load a different directory. By default the daemon
looks for `skills/` three levels above its executable, which resolves to the
repository's `skills/` directory for a standard Cargo build.

## Run and evaluate

```sh
./target/release/chauffeur daemon
./target/release/chauffeur skills
./target/release/chauffeur skill evaluate --skill workspace-edit-gate --context context.json
```

`skill evaluate` sends a bounded `SkillContext` to the running daemon and prints
the status, branch, effect, reminder, and missing evidence as JSON. The
OpenCode adapter submits matching permission events automatically through
`permission.evaluate`.

Example context file:

```json
{
  "event": "permission.evaluate",
  "action": "edit",
  "agent_id": "demo-session",
  "occurred_at": 1780000000,
  "state": "The user requested an edit to README.md.",
  "evidence": {
    "resource_present": true,
    "resource_within_workspace": true,
    "resource_named_by_user": true
  }
}
```

The daemon checks all deterministic evidence before consulting Laya. Missing
evidence returns the contract's fixed `missing_evidence` outcome. Low Laya
confidence selects `uncertain`; Laya socket or response failures select
`judge_failure`. These outcomes can ask for review without generating text.

## Author a skill

Add a `.json` file under `skills/` with these required sections:

1. `schema_version` and `identity` (`id`, `name`, numeric `major.minor.patch` `version`)
2. `match.events` and `match.actions` with exact host values
3. `predicates.required_evidence` and `predicates.forbidden_evidence`
4. A typed `decision` (`choice`, `score`, or `noul`) with confidence and explicit
   numeric thresholds or criterion-to-branch mappings
5. Six static outcomes: `positive`, `negative`, `uncertain`,
   `missing_evidence`, `judge_failure`, and `cooldown`
6. `cooldown_seconds`

All objects reject unknown fields. Skill files may be at most 65,536 bytes; a
directory may contain at most 64 JSON skill files and 256 entries total. Skill
contexts are limited to 65,536 encoded bytes, and the daemon caps RPC request
bodies at 128 KiB. See
[`skills/workspace-edit-gate.json`](./skills/workspace-edit-gate.json) for a
complete permission-boundary example.

## Configuration

| Variable | Default |
|---|---|
| `CHAUFFEUR_SKILLS_DIR` | `skills/` three levels above the executable |
| `CHAUFFEUR_DAEMON_URL` | `http://127.0.0.1:18788` |
| `CHAUFFEUR_DAEMON_TOKEN` | unset |
| `CHAUFFEUR_STATE_DIR` | `~/.local/state/chauffeur` |
| `LAYA_SOCKET` | `~/Library/Application Support/laya/laya.sock` |
| `CHAUFFEUR_BIN` | `chauffeur` for the OpenCode adapter |
| `CHAUFFEUR_IDLE_STEERING` | `false`; set to `true` to enable idle reminders |

## Development checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
cd adapters/opencode && npm run typecheck
```
