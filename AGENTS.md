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
cargo build --locked --release --bin singulari-world --bin singulari-world-mcp

target/release/singulari-world --store-root .world-store host-worker \
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
target/release/singulari-world --store-root .world-store vn-serve --port 4177
```

Open:

```text
http://127.0.0.1:4177/
```

For phone play over Tailscale, use the same web app and pass only a Tailscale
address or hostname:

```bash
target/release/singulari-world --store-root .world-store vn-serve \
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
- `worldsim_current_cg_image`
- `worldsim_probe_image_ingest`
- `worldsim_complete_visual_job_from_base64`
- `worldsim_complete_visual_job_from_url`
- `worldsim_claim_visual_job`
- `worldsim_complete_visual_job`
- `worldsim_release_visual_job`
- `worldsim_resume_pack`
- `worldsim_search`
- `worldsim_codex_view`
- `worldsim_validate`
- `worldsim_repair_db`

## ChatGPT Web MCP

`singulari-world-mcp-web` serves the same MCP handler over Streamable HTTP for
remote ChatGPT app hosts:

```bash
cargo build --locked --release --bin singulari-world-mcp-web
target/release/singulari-world-mcp-web --host 127.0.0.1 --port 4187 --path /mcp --profile play
```

ChatGPT web requires a remote HTTPS URL; loopback is for local smoke tests or a
trusted tunnel/reverse proxy. The default `play` profile exposes player-visible
read tools, player input submission, `worldsim_current_cg_image`, and
`worldsim_probe_image_ingest`, and the narrow
`worldsim_complete_visual_job_from_base64` /
`worldsim_complete_visual_job_from_url` PNG completion paths. It does not expose
hidden pending-turn packets, direct commits, generic visual claim completion
from local paths, DB repair, or other trusted local-agent tools. Use
`--profile trusted-local` only behind an operator-controlled private boundary.

Image direction is probe-first. `worldsim_current_cg_image` returns an existing
stored PNG as MCP image content. `worldsim_probe_image_ingest` records only the
shape of image references a host can pass back (`image_base64`, `image_url`,
`resource_uri`, `file_id`) and deliberately does not persist image bytes or
complete visual jobs. `worldsim_complete_visual_job_from_base64` accepts only
PNG base64 or `data:image/png;base64,...`, stages it temporarily, and then
reuses the normal visual-job completion verifier.
`worldsim_complete_visual_job_from_url` accepts only HTTPS `image/png` URLs,
rejects local/private hosts, private DNS resolution targets, and embedded
credentials, follows at most three redirects, caps the body at 16 MiB, and then
uses the same verifier.

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

If the host starts `host-worker` through `launchctl`, pass a full `--codex-bin`
path and include a PATH that can resolve `node`. The npm Codex launcher uses
`#!/usr/bin/env node`, so a minimal launchd PATH can fail even when the same
command works in an interactive shell.

`world_id -> thread_id` is the durable realtime context contract. The websocket
URL is replaceable runtime plumbing; the saved thread is the world's narrative
working context. The default `--codex-thread-context-mode native-thread`
includes prior app-server turns for prose rhythm and immediate continuity while
injecting only a compact authoritative world packet for current state, hidden
adjudication, and output contract. Use `bounded-packet` only when thread history
should be excluded and the full pending packet reinjected every turn. If
app-server `thread/resume` fails for a stale or missing thread, clear only that
world's binding and let the next dispatch rebuild from the world store.

## Visual Job Worker

Image jobs are host-consumed jobs, not `codex exec` jobs. The same
`host-worker` started by `싱귤러리 월드 준비해줘` owns the app-server image loop:
claim one job, request Codex App `imageGeneration`, copy the returned
`savedPath`, then complete the job. The Codex chat/session-level `image_gen`
path is not an acceptable substitute.

Turn CG retry is a regeneration request. If a current turn image already exists,
the retry marker still creates a new `turn_cg:<turn_id>` job; completion
overwrites the turn PNG, clears the visual claim, and removes the retry marker.

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

Older local worlds created before initial VN render packets existed may validate
but fail `vn-packet` with a missing `sessions/<session>/render_packets/turn_0000.json`.
For those worlds, add the initial waiting render packet from the current seed
contract or recreate the world from seed, then run `repair-db`.

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
