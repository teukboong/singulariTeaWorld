# WebGPT MCP Activation Guide

This guide activates WebGPT as a `singulari-world` backend behind the shared VN
browser app. It covers both narrative text and image generation. The browser UI
does not change; only the worker engine changes.

## Runtime Shape

```text
vn-serve browser UI
  -> world-store pending text / visual jobs
  -> host-worker
       text lane  -> WebGPT MCP webgpt_research
       image lane -> WebGPT MCP webgpt_generate_image
  -> validated world-store commit / PNG completion
```

The Rust world store remains source of truth. WebGPT receives a player-visible
and schema-bound packet, returns exactly one response object or one generated
PNG, and the worker commits through the Rust validators.

## Requirements

- The bundled `webgpt-mcp-checkout/` runtime in this repository, or an explicit
  `SINGULARI_WORLD_WEBGPT_MCP_WRAPPER` path when intentionally testing another
  checkout.
- Chrome or a compatible Chromium browser available to the WebGPT MCP worker.
- A ChatGPT login available in the browser profiles used by WebGPT.
- Human handling for ChatGPT or Cloudflare challenges when they appear.

Build the local binaries first:

```bash
cargo build --locked --release \
  --bin singulari-world \
  --bin singulari-world-mcp-web
```

By default the worker uses the bundled wrapper:

```bash
webgpt-mcp-checkout/scripts/webgpt-mcp.sh
```

On a fresh clone, prepare that bundled runtime once:

```bash
scripts/setup-webgpt-runtime.sh
```

This installs only the bundled `chatgpt-worker` dependencies, rebuilds its
`dist/index.js`, and builds the vendored `webgpt-mcp` Rust binary.

If you intentionally want to test another checkout, point the worker at that wrapper:

```bash
SINGULARI_WORLD_WEBGPT_MCP_WRAPPER=/absolute/path/to/other-webgpt-mcp/scripts/webgpt-mcp.sh
```

## Session Model

WebGPT text and WebGPT image work are separate lanes from process start. They
must not share one browser tab, one CDP port, or one profile directory.

Default lanes:

| Lane | Tool | CDP port | Profile dir | World binding |
| --- | --- | --- | --- | --- |
| Text | `webgpt_research` | `9238` | `~/.hesperides/singulari-world/webgpt/text-profile` | `agent_bridge/webgpt_conversation_binding.json` |
| Turn CG image | `webgpt_generate_image` | `9239` | `~/.hesperides/singulari-world/webgpt/image-profile` | `agent_bridge/webgpt_image_conversation_binding.json` |
| Reference image | `webgpt_generate_image` | `9240` | `~/.hesperides/singulari-world/webgpt/reference-image-profile` | `agent_bridge/webgpt_reference_asset_conversation_binding.json` |

Override only when a local port/profile collides:

```bash
SINGULARI_WORLD_WEBGPT_TEXT_CDP_PORT=9238
SINGULARI_WORLD_WEBGPT_IMAGE_CDP_PORT=9239
SINGULARI_WORLD_WEBGPT_REFERENCE_IMAGE_CDP_PORT=9240
SINGULARI_WORLD_WEBGPT_TEXT_PROFILE_DIR="$HOME/.hesperides/singulari-world/webgpt/text-profile"
SINGULARI_WORLD_WEBGPT_IMAGE_PROFILE_DIR="$HOME/.hesperides/singulari-world/webgpt/image-profile"
SINGULARI_WORLD_WEBGPT_REFERENCE_IMAGE_PROFILE_DIR="$HOME/.hesperides/singulari-world/webgpt/reference-image-profile"
```

Startup fails if any lanes share a port or profile. That is intentional:
shared sessions contaminate text/image continuity and create one browser queue
for two different jobs.

## Start Local Play

Start the VN app:

```bash
target/release/singulari-world --store-root .world-store vn-serve --port 4177
```

Start the worker with WebGPT for both lanes:

```bash
target/release/singulari-world --store-root .world-store host-worker \
  --text-backend webgpt \
  --visual-backend webgpt \
  --interval-ms 750
```

Open the browser UI:

```text
http://127.0.0.1:4177/
```

The new-world dialog writes a locked backend selection file:

```text
worlds/<world-id>/agent_bridge/backend_selection.json
```

That file owns the chosen backend pair for the life of the world. To play Codex
text plus WebGPT image, choose that pair when creating the world. Do not switch
platforms inside an existing world; create a new world with a different backend
selection.

## Optional Prewarm

Usually `host-worker` starts the WebGPT browser sessions only when needed. To
prewarm or manually solve a login/challenge before play, start the lane sessions
directly:

```bash
WEBGPT_MCP_CDP_PORT=9238 \
WEBGPT_MCP_MANUAL_PROFILE_DIR="$HOME/.hesperides/singulari-world/webgpt/text-profile" \
  /absolute/path/to/webgpt-mcp-checkout/scripts/webgpt-cdp-session.sh start

WEBGPT_MCP_CDP_PORT=9239 \
WEBGPT_MCP_MANUAL_PROFILE_DIR="$HOME/.hesperides/singulari-world/webgpt/image-profile" \
  /absolute/path/to/webgpt-mcp-checkout/scripts/webgpt-cdp-session.sh start
```

The CDP helper checks whether the configured endpoint is already alive. It
starts Chrome only when the lane endpoint is absent, and it fails if another
process owns the port. On macOS it starts Chrome minimized and minimizes the
window again after CDP becomes reachable; CDP input does not require the window
to stay foregrounded.

Lane profiles are seeded from reusable logged-in profile sources before launch.
By default the helper checks `~/.hesperides/chatgpt-chrome-profile-manual` and
the normal Chrome user data dir. Override the source list with:

```bash
WEBGPT_MCP_BOOTSTRAP_SOURCE_PROFILE_DIRS="/path/to/source-a:/path/to/source-b"
```

If a matching Chrome process remains but its CDP endpoint is dead, the helper
reaps that stale process before trying to reopen the lane. A port held by an
unrelated process still fails loudly.

## Continuity Policy

WebGPT gets an active revival packet because ChatGPT project/session compaction
is less explicit than the world store. Each text turn includes:

- compact current pending-turn contract
- larger `resume_pack` windows
- player-visible Archive View
- query recall hits
- recent entity and relationship updates from `world.db`

The image lane keeps browser/profile isolation from the text lane, and splits
its own ChatGPT conversations by asset class. Turn CG reuses
`agent_bridge/webgpt_image_conversation_binding.json`; character sheets,
location sheets, and other reference assets reuse
`agent_bridge/webgpt_reference_asset_conversation_binding.json`. Reference
assets are source material only. They should not pollute the turn-CG
conversation and must never be surfaced as the scene CG.
When a turn-CG job has accepted `reference_paths`, the worker passes those
paths to `webgpt_generate_image` as `reference_paths`; the WebGPT browser worker
uploads the actual image files as attachments before sending the scene prompt.
Textual path notes remain audit context only.
When both text and image backends are WebGPT, the worker dispatches the two
lanes in parallel from the same tick instead of waiting for the pending text
turn to finish before claiming visual jobs.

## Image Rules

WebGPT image generation closes the repo-owned visual jobs:

```text
claim visual job -> webgpt_generate_image -> extracted PNG -> complete job
```

Turn CG has a major-character design gate. Before a protagonist or anchor-level
character has an accepted character sheet in `assets/vn/character_sheets/`, the
scene CG prompt forbids direct depiction and asks for POV, environment-only,
off-screen, shadow, or cropped non-identifying framing. Character sheet jobs may
still depict the character, but they stay in the reference-asset conversation
and are saved only to their reference asset paths; the gate applies to scene CG.

WebGPT image cadence defaults to every 2 turns:

```bash
SINGULARI_WORLD_WEBGPT_TURN_CG_CADENCE_MIN=2
```

## Verification

Check the worker exposes the WebGPT lane options:

```bash
target/release/singulari-world host-worker --help | rg 'webgpt-(text|image)'
```

Check the CDP helper script parses:

```bash
bash -n /absolute/path/to/webgpt-mcp-checkout/scripts/webgpt-cdp-session.sh
```

Smoke the Rust targets:

```bash
cargo check --locked --bin singulari-world --bin singulari-world-mcp-web
```

During runtime, watch host-worker JSONL events. WebGPT text dispatch events
include the prompt/response files; WebGPT image completion events include the
generated PNG path. The dispatch record should include lane-specific MCP CDP
fields such as `mcp_cdp_port` and `mcp_profile_dir`.

## Troubleshooting

- Missing wrapper: set `SINGULARI_WORLD_WEBGPT_MCP_WRAPPER` to
  `webgpt-mcp-checkout/scripts/webgpt-mcp.sh`.
- Login or bot challenge: solve it in the lane's browser window, then let the
  worker retry the same pending job.
- Port owned by another process: either stop that process or set the lane's CDP
  port env var. Do not point text and image to the same port.
- Wrong profile reused: set distinct text/image profile dirs, then restart the
  worker.
- Browser window closed but port still blocked: rerun the lane start; the CDP
  helper reaps matching stale sessions before relaunching.
- Text-only validation: use `--visual-backend none`; this leaves visual jobs
  queued and does not mutate them.
