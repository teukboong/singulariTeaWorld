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
scripts/privacy-audit.sh
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

## Codex App Prep

When the operator says `싱귤러리 월드 준비해줘` from Codex App, prepare the
background runtime, not a one-off chat turn.

From this repository, build the binary and start one long-running worker:

```bash
cargo build --locked --bin singulari-world

target/debug/singulari-world --store-root .world-store host-worker \
  --interval-ms 750
```

That worker starts a managed loopback `codex app-server` when no
`--codex-app-server-url` is provided. It consumes pending text turns and visual
jobs through Codex App app-server only. Visual jobs must close through
`imageGeneration -> savedPath -> complete`; do not route them through this
Codex chat's visual generation session. Keep Codex App open while playing; idle
worker ticks spend zero model tokens and wait for browser-created work.

After prep, the only user-facing runtime that still needs to run is the VN app:

```bash
target/debug/singulari-world --store-root .world-store vn-serve --port 4177
```

Open:

```text
http://127.0.0.1:4177/
```

For phone play over Tailscale, use the same web app and pass only a Tailscale
address or hostname:

```bash
target/debug/singulari-world --store-root .world-store vn-serve \
  --host <tailscale-ip-or-hostname> \
  --port 4177
```

Do not bind the VN server to `0.0.0.0` for convenience. The server allowlist is
loopback plus Tailscale, so normal LAN exposure should fail closed.

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

For normal Codex App play, prefer the prep command above. Manual
`agent-submit` / `agent-next` / `agent-commit` commands are debugging tools.

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
run the host worker with the realtime app-server backend:

```bash
singulari-world host-worker \
  --interval-ms 750
```

`host-worker` is the app-facing supervisor. It uses the official Codex
app-server websocket and spends zero model tokens while idle. If no explicit
websocket URL is provided, it
starts a managed loopback `codex app-server`, records the runtime URL in the
store-root `agent_bridge` directory, and stops the child when the worker exits.
`host-worker` reads
`worlds/<world-id>/agent_bridge/codex_thread_binding.json` on every tick, so
rebinding does not require restarting the worker.
When no active world exists, `host-worker` waits instead of failing, so the app
can start it before the user creates or loads a world.

`world_id -> thread_id` is the durable realtime context contract. The websocket
URL is replaceable runtime plumbing; the saved thread is the world's narrative
working context. Codex may compact that thread normally, so every dispatched
turn must still include the bounded world-store packet. If app-server
`thread/resume` fails for a stale or missing thread, clear only that world's
binding and let the next dispatch rebuild from the world store.

## Visual Job Worker

Image jobs are host-consumed jobs, not `codex exec` jobs. The same
`host-worker` started by `싱귤러리 월드 준비해줘` owns the app-server image loop:
claim one job, request Codex App `imageGeneration`, copy the returned
`savedPath`, then complete the job. The Codex chat/session-level `image_gen`
path is not an acceptable substitute.

The MCP path uses the standalone `singulari-world-mcp` tool surface.
`worldsim_claim_visual_job` returns structured MCP content containing
`job.codex_app_call`. Codex App consumes that structured call with its built-in
image generation capability, writes the PNG to `destination_path`, then calls
`worldsim_complete_visual_job`. This is the repo-owned MCP contract; it does not
depend on `~/.codex` skills, external provider keys, or the active chat visual
session.

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
- realtime Codex app-server and CLI thread dispatch
- host worker supervisor contract
- visual job app-server and MCP completion contracts
- privacy audit gate for tracked files and git history
- release and smoke scripts

Still host-owned:

- keeping Codex App open while the worker uses app-server
- app-managed start/stop of `host-worker`
- packaged installers for macOS/Windows/Linux
