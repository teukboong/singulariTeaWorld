# Deployment

This repository is designed to be published without private world state.

## Public Repository Checklist

- Keep `Cargo.lock` committed.
- Keep `.world-store/`, `target/`, `dist/`, generated images, local DB files, and
  personal world exports out of git.
- Keep local-only scrub terms in `.privacy-denylist` or
  `SINGULARI_PRIVACY_DENYLIST_INLINE`; both are consumed by
  `scripts/privacy-audit.sh` without committing private terms.
- Run the full release gate before pushing:

```bash
scripts/release-build.sh
```

- Run the local smoke test before tagging a public alpha:

```bash
scripts/smoke-local.sh
```

## Build

```bash
cargo build --locked --release
```

Release binaries:

- `target/release/singulari-world`
- `target/release/singulari-world-mcp`

## Codex MCP Install

Use the helper:

```bash
scripts/install-codex-mcp.sh
```

Or install manually:

```bash
cargo build --locked --release --bin singulari-world-mcp
codex mcp add singulari-world -- "$(pwd)/target/release/singulari-world-mcp"
codex mcp get singulari-world
```

To force a specific local store:

```bash
SINGULARI_WORLD_HOME="$HOME/.local/share/singulari-world" \
  scripts/install-codex-mcp.sh
```

## Local VN Runtime

Create a world:

```bash
cargo run --locked --bin singulari-world -- start \
  --seed-text "fantasy, modern reincarnation, gifted protagonist" \
  --json
```

Serve the VN UI:

```bash
cargo run --locked --bin singulari-world -- vn-serve --port 4177
```

Open:

```text
http://127.0.0.1:4177/
```

## Background Worker Contract

The browser writes durable pending jobs. It does not call an LLM directly.

Text turns:

```bash
codex app-server --listen ws://127.0.0.1:<port>

singulari-world host-worker \
  --world-id <world-id> \
  --text-backend codex-app-server \
  --codex-app-server-url ws://127.0.0.1:<port> \
  --interval-ms 750
```

The intended packaged-app backend is `codex-app-server`, where the host starts
or receives a Codex app-server websocket URL and dispatches only when a pending
world turn exists. `codex-exec-resume` remains the on-demand CLI backend for
hosts that do not run a websocket app-server.

Image jobs:

```bash
singulari-world visual-job-claim --world-id <world-id> --json
# Codex App host image generation saves PNG to claim.job.destination_path.
singulari-world visual-job-complete \
  --world-id <world-id> \
  --slot <slot> \
  --claim-id <claim-id> \
  --json
```

On image-generation host failure:

```bash
singulari-world visual-job-release --world-id <world-id> --slot <slot> --json
```

## Current Alpha Boundary

The standalone simulator owns:

- world persistence
- VN projection
- MCP tools
- text-turn pending/commit
- Codex thread binding
- host-worker event supervisor
- image job claim/complete/release contracts

The embedding host still owns:

- starting/stopping `agent-watch`
- starting/stopping `host-worker`
- starting/stopping the Codex app-server websocket
- passing `--codex-app-server-url` to `host-worker`
- consuming `visual_job_pending`
- calling its image generation capability
- saving PNG files to the returned destination paths
