# Singulari World Agent Guide

This repository is a standalone, public-safe text world simulator with a local
VN web projection and MCP tools. Start here after cloning.

## Boundaries

- Do not commit local world stores, generated images, DB files, or private
  narrator/world presets.
- Runtime state belongs in `.world-store/`, `SINGULARI_WORLD_HOME`, or an
  explicit export chosen by the user.
- Browser-visible packets must stay player-visible only. Hidden adjudication
  context is for trusted local agents and must not leak into visible prose,
  Codex View, image prompts, or logs.
- The simulator does not call external image APIs directly. It emits redacted
  image jobs; the embedding host owns image generation and PNG save.

## First Setup

```bash
cargo build --locked
scripts/smoke-local.sh
```

Expected result:

```text
smoke ok: world_id=<id> store_root=<temp-dir>
```

## Main Checks

Run before publishing or after changing runtime behavior:

```bash
cargo fmt --all -- --check
cargo check --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
```

Or run the bundled gate:

```bash
scripts/release-build.sh
```

## CLI Basics

Create a world:

```bash
cargo run --locked --bin singulari-world -- start \
  --seed-text "fantasy, modern reincarnation, gifted protagonist" \
  --json
```

Serve the VN app:

```bash
cargo run --locked --bin singulari-world -- vn-serve --port 4177
```

Open:

```text
http://127.0.0.1:4177/
```

Use a specific store:

```bash
SINGULARI_WORLD_HOME="$HOME/.local/share/singulari-world" \
  cargo run --locked --bin singulari-world -- active --json
```

## MCP Install

Install into Codex:

```bash
scripts/install-codex-mcp.sh
codex mcp get singulari-world
```

Manual equivalent:

```bash
cargo build --locked --release --bin singulari-world-mcp
codex mcp add singulari-world -- "$(pwd)/target/release/singulari-world-mcp"
```

Core MCP tools:

- `worldsim_start_world`
- `worldsim_current`
- `worldsim_submit_player_input`
- `worldsim_next_pending_turn`
- `worldsim_commit_agent_turn`
- `worldsim_visual_assets`
- `worldsim_claim_visual_job`
- `worldsim_complete_visual_job`
- `worldsim_release_visual_job`
- `worldsim_resume_pack`
- `worldsim_search`
- `worldsim_codex_view`
- `worldsim_validate`
- `worldsim_repair_db`

## Agent-Authored Text Turns

The browser queues player input; a trusted local agent commits the visible turn.

Queue input:

```bash
singulari-world agent-submit --world-id <world-id> --input 1 --json
```

Read pending packet:

```bash
singulari-world agent-next --world-id <world-id> --json
```

Commit an agent response:

```bash
singulari-world agent-commit \
  --world-id <world-id> \
  --response <agent-response.json> \
  --json
```

For realtime Codex thread dispatch, bind a world to the active Codex thread and
run the watcher:

```bash
singulari-world codex-thread-bind \
  --world-id <world-id> \
  --thread-id <codex-thread-id> \
  --codex-bin "$(command -v codex)" \
  --json

singulari-world agent-watch --world-id <world-id> --interval-ms 750
```

`agent-watch` reads
`worlds/<world-id>/agent_bridge/codex_thread_binding.json` on every tick, so
rebinding does not require restarting the watcher.

## Visual Job Worker

Image jobs are host-consumed jobs, not `codex exec` jobs.

Claim one job:

```bash
singulari-world visual-job-claim --world-id <world-id> --json
```

The host should run its image generation capability with:

- `claim.job.prompt`
- `claim.job.reference_paths`
- `claim.job.destination_path`

Then complete:

```bash
singulari-world visual-job-complete \
  --world-id <world-id> \
  --slot <slot> \
  --claim-id <claim-id> \
  --json
```

If the host wrote a temporary PNG instead of writing directly to
`destination_path`:

```bash
singulari-world visual-job-complete \
  --world-id <world-id> \
  --slot <slot> \
  --claim-id <claim-id> \
  --generated-path <generated.png> \
  --json
```

If generation fails or is cancelled:

```bash
singulari-world visual-job-release --world-id <world-id> --slot <slot> --json
```

## Export, Import, Repair

```bash
singulari-world export-world --world-id <world-id> --output <bundle-dir> --json
singulari-world import-world --bundle <bundle-dir> --activate --json
singulari-world validate --world-id <world-id> --json
singulari-world repair-db --world-id <world-id> --json
```

## Public Alpha Status

Implemented:

- file-backed world store
- SQLite projection and FTS search
- local VN server
- MCP server
- agent pending/commit loop
- realtime Codex thread binding fallback
- visual job claim/complete/release contract
- release and smoke scripts

Still host-owned:

- app-managed start/stop of `agent-watch`
- actual Codex App image-generation invocation
- packaged installers for macOS/Windows/Linux
