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
codex app-server --listen ws://127.0.0.1:<port>

cargo run --bin singulari-world -- host-worker \
  --interval-ms 750 \
  --world-id "<world-id>" \
  --text-backend codex-app-server \
  --codex-app-server-url "ws://127.0.0.1:<port>"
```

`host-worker` is the cross-platform process an embedding app should start and
stop with the VN app. Its primary realtime text backend is `codex-app-server`,
which talks to the official Codex app-server websocket and starts a model turn
only when a pending world turn exists. `codex-exec-resume` remains the
on-demand CLI backend for hosts that do not run an app-server websocket. Image
jobs are queue-based too: Codex App consumes the redacted
`codex_app.image.generate` job and saves the PNG to the returned
`destination_path`.

Each world owns a durable Codex `thread_id` under
`worlds/<world-id>/agent_bridge/codex_thread_binding.json`. That thread keeps
the warm narrative context; the world DB remains source of truth and is injected
into every turn so Codex compaction or thread rebuilds do not erase canon.

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

Run the privacy audit before publishing:

```bash
scripts/privacy-audit.sh
```

For personal names, emails, private project names, or local-only lore, put one
literal term per line in `.privacy-denylist`. That file is ignored by git and is
also honored by `scripts/release-build.sh`.

## Release Gate

```bash
scripts/release-build.sh
```

See [docs/deployment.md](docs/deployment.md) for the public alpha deployment
checklist, MCP install flow, and background worker contract. See
[docs/host-worker.md](docs/host-worker.md) for the host lifecycle and backend
contract.
