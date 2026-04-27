# Singulari World

Standalone file-backed text world simulator with:

- durable per-world JSON/JSONL records
- SQLite projections and full-text search
- visual-novel web projection
- local MCP tools for agent-authored narrative turns
- hidden/player-visible redaction boundaries

This repository is public-safe by default. Personal narrator profiles, private
world presets, local memories, and relationship-specific lore should live outside
the repository and be provided at runtime as seeds or local configuration.

For agent operators, start with [AGENTS.md](AGENTS.md). It contains the clone
setup path, verification commands, MCP install command, VN runtime commands, and
background worker contracts.

## Quick Start

```bash
scripts/smoke-local.sh

cargo run --bin singulari-world -- start \
  --seed-text "<world-seed>" \
  --json

cargo run --bin singulari-world -- vn-serve --port 4177
```

Agent-authored VN flow:

```bash
SINGULARI_WORLD_AGENT_BRIDGE=1 cargo run --bin singulari-world -- vn-serve --port 4177
```

Background job bridge for packaged apps:

```bash
cargo run --bin singulari-world -- agent-watch --once
cargo run --bin singulari-world -- agent-watch --interval-ms 1500
cargo run --bin singulari-world -- codex-thread-bind \
  --world-id "<world-id>" \
  --thread-id "$CODEX_THREAD_ID" \
  --codex-bin "$(command -v codex)"
cargo run --bin singulari-world -- agent-watch \
  --interval-ms 750 \
  --world-id "<world-id>"
```

`agent-watch` is the cross-platform process Codex App should start and stop with
the VN app. It emits JSONL events for pending narrative turns and visual asset
jobs. Bind a world to the active Codex thread once with `codex-thread-bind`; the
watcher reads that binding every tick and immediately dispatches pending
narrative turns through `codex exec resume <thread> -`, so the active Codex
thread can author and commit the scene without a cron/heartbeat delay. Passing
`--codex-thread-id` to `agent-watch` is still supported as a bootstrap shortcut:
it seeds or refreshes the durable binding for the watched world. Image jobs
target Codex App's built-in image generation capability
(`codex_app.image.generate`) and must be saved to the returned
`destination_path`.

The MCP server runs over stdio:

```bash
cargo run --bin singulari-world-mcp
```

Install it into Codex:

```bash
scripts/install-codex-mcp.sh
```

The MCP surface includes `worldsim_visual_assets`, which returns the same
player-visible manifest and Codex App image generation jobs without requiring a
separate image provider. Codex App should claim and complete those jobs through
`worldsim_claim_visual_job` and
`worldsim_complete_visual_job` or their CLI equivalents. If generation fails,
release the claim with `worldsim_release_visual_job` / `visual-job-release`:

```bash
cargo run --bin singulari-world -- visual-job-claim --world-id "<world-id>" --json
# run Codex App image generation with the returned prompt and save the PNG
cargo run --bin singulari-world -- visual-job-complete \
  --world-id "<world-id>" \
  --slot "<slot>" \
  --claim-id "<claim-id>" \
  --json
```

## Seed Anchor

World seeds may define an `anchor_character` when a run needs a persistent
counterpart, companion, rival, patron, or other core character. The default is
generic; personal narrator identities and relationship-specific roles should be
provided by private seeds or local config.

## Storage

By default the world store is under the user data directory implied by the
runtime. Override it explicitly with:

```bash
SINGULARI_WORLD_HOME=/path/to/world-store cargo run --bin singulari-world -- active --json
```

## Public Boundary

Do not commit private presets, real conversation memories, generated world saves,
or local image assets. Fictional world state belongs in a local world store or an
explicit export bundle chosen by the user.

## Release Gate

```bash
scripts/release-build.sh
```

See [docs/deployment.md](docs/deployment.md) for the public alpha deployment
checklist, MCP install flow, and background worker contract.
