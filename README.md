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

cargo run --locked --bin singulari-world -- start \
  --seed-text "<world-seed>" \
  --json

cargo run --locked --bin singulari-world -- vn-serve --port 4177
```

## Codex App Play Mode

The intended local play flow is:

1. Open Codex App.
2. Tell the agent: `싱귤러리 월드 준비해줘`.
3. Let the agent start the background worker below.
4. Run or open the VN web app and keep Codex App open while playing.

Prep command:

```bash
cargo build --locked --release --bin singulari-world --bin singulari-world-mcp

target/release/singulari-world --store-root .world-store host-worker \
  --interval-ms 750
```

Then start the VN app:

```bash
target/release/singulari-world --store-root .world-store vn-serve --port 4177
```

Open:

```text
http://127.0.0.1:4177/
```

`host-worker` is the cross-platform process an embedding app should start
before the VN app needs agent-authored turns. It talks to the official Codex app-server websocket
and starts a model turn only when a pending world turn exists. If no
`--codex-app-server-url` is provided, the worker starts `codex app-server` on a
loopback port, records the runtime URL under the store-root `agent_bridge`
directory, and stops it when the worker exits. Keep Codex App open while the web
app is in use. Idle ticks spend zero model tokens.

The worker owns the Codex App app-server loop for both text turns and visual
jobs. It claims one pending visual job per tick, asks Codex App for one
`imageGeneration`, copies the returned PNG to `destination_path`, and completes
the job. It never uses external providers, local placeholders, `~/.codex`
skills, or the active chat visual session.

For automatic image completion, the installed `singulari-world-mcp` server is
the standalone bridge. `worldsim_claim_visual_job` returns structured MCP
content with `job.codex_app_call`; Codex App consumes that structured call with
its built-in image generation capability, writes the PNG to `destination_path`,
then calls `worldsim_complete_visual_job`.

Each world owns a durable Codex `thread_id` under
`worlds/<world-id>/agent_bridge/codex_thread_binding.json`. That thread keeps
the warm narrative context. By default, `host-worker` resumes it in
`native-thread` mode so Codex App history carries prose rhythm and immediate
continuity, while each turn injects a compact authoritative world packet for
current state, hidden adjudication, and output contract. The world DB remains
source of truth, so Codex compaction or thread rebuilds do not erase canon.
When no active world exists yet, `host-worker` idles until the browser creates
or loads one.

For phone play over Tailscale, use the same VN web app, not a separate mobile
URL:

```bash
target/release/singulari-world --store-root .world-store vn-serve \
  --host <tailscale-ip-or-hostname> \
  --port 4177
```

The VN server allowlist accepts loopback and Tailscale addresses only. Do not
use `0.0.0.0` as a shortcut.

The local Codex MCP server runs over stdio:

```bash
cargo run --locked --bin singulari-world-mcp
```

Install it into Codex:

```bash
scripts/install-codex-mcp.sh
```

The MCP surface includes `worldsim_visual_assets`, which returns the same
player-visible manifest and Codex App image generation jobs without requiring a
separate image provider. `worldsim_claim_visual_job` also includes the current
turn CG job from the VN packet when it is pending. Codex App should claim and
complete those jobs through `worldsim_claim_visual_job` and
`worldsim_complete_visual_job`. The CLI claim/complete commands remain operator
inspection tools, not the normal play loop.

For ChatGPT web developer mode, serve the remote MCP transport separately:

```bash
cargo run --locked --bin singulari-world-mcp-web -- \
  --host 127.0.0.1 \
  --port 4187 \
  --path /mcp \
  --profile play
```

ChatGPT web cannot connect to a private loopback URL directly; put this behind a
trusted HTTPS tunnel or reverse proxy and paste the public `/mcp` URL into the
custom app form. The default `play` profile exposes player-visible read tools,
player input submission, current-CG image output, and
`worldsim_probe_image_ingest`, plus the narrow
`worldsim_complete_visual_job_from_base64` /
`worldsim_complete_visual_job_from_url` PNG completion paths. It does not expose
trusted local-agent tools such as pending hidden adjudication, direct commit, or
generic completion from local paths. Existing generated CGs can be returned to
the host as MCP image content through `worldsim_current_cg_image`.
The probe tool records which image reference shapes the host can pass back
(`image_base64`, `image_url`, `resource_uri`, or `file_id`) without persisting
image bytes. If the host can pass PNG bytes, complete the pending visual job
with `worldsim_complete_visual_job_from_base64` using raw base64 or a
`data:image/png;base64,...` URL. If it can pass a temporary URL, use
`worldsim_complete_visual_job_from_url`; the server accepts only HTTPS
`image/png`, rejects local/private hosts, private DNS resolution targets, and
credentials, limits redirects, and caps the download size.

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
