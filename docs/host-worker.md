# Host Worker Contract

`singulari-world` keeps the simulator standalone. It owns world storage,
pending-turn records, visual job records, and redacted prompts. The VN browser
app is the common play frontend; the embedding host owns the selected narrative
engine and image generation.

`host-worker` is the execution tick between those systems. In the default VN
deployment, `vn-serve` starts `host-worker --once` whenever the browser creates
new text or visual work, so the user does not have to keep a long-running worker
alive with `launchd`/KeepAlive. A long-running worker remains a diagnostic and
custom-host mode, not the required play path.

For the complete WebGPT setup checklist, including ChatGPT login/challenge
handling and separate text/image browser lanes, see
[webgpt-mcp-activation.md](webgpt-mcp-activation.md).

## Codex App Operator Prep

When a Codex App operator says `싱귤러리 월드 준비해줘`, build the release
binary, start `vn-serve`, and keep Codex App open while the VN browser is used:

```bash
cargo build --locked --release --bin singulari-world --bin singulari-world-mcp
target/release/singulari-world --store-root .world-store vn-serve --port 4177
```

The web app writes pending jobs into the world store and wakes a one-shot
worker. That one-shot worker prepares the managed loopback `codex app-server`
only when a pending Codex App text or image job actually exists, consumes one
bounded tick of work, writes the visible result back, and exits. Idle browser
time does not start model turns.

For diagnostics, the same tick can be run by hand:

```bash
singulari-world --store-root .world-store host-worker --world-id <world-id> --once
```

The long-running loop form is optional and meant for custom embedding hosts that
explicitly want external supervision.

## Runtime Path

`host-worker` has two text dispatch paths behind the same VN web frontend.

The default `codex-app-server` backend:

- Connect to an explicit `--codex-app-server-url`, or start a managed loopback
  `codex app-server`.
- Read the active world from `.world-store/active_world.json`.
- When `pending_turn.json` exists, resume the saved world thread or start a new
  Codex thread.
- Send a prompt through `turn/start`. In the default `native-thread` mode, the
  prompt keeps the Codex App thread history and injects only a compact
  authoritative world packet; `bounded-packet` excludes prior app-server turns
  and reinjects the full pending packet.
- Persist the returned `thread_id` in
  `worlds/<world-id>/agent_bridge/codex_thread_binding.json`.
- Commit the agent-authored visible turn through the normal world-store
  contract.

That thread is the world's warm narrative context. The world DB and JSON/JSONL
files remain source of truth; the compact authoritative packet wins over thread
memory whenever they conflict, so Codex compaction or thread rebuilds do not
erase canon.

If `thread/resume` fails for a stale or missing thread, the worker clears only
that world's binding. The next tick starts a fresh thread from the same world
store.

The `webgpt` backend:

- Finds a sibling `webgpt-mcp-checkout/scripts/webgpt-mcp.sh`, or uses
  `--webgpt-mcp-wrapper` / `SINGULARI_WORLD_WEBGPT_MCP_WRAPPER` in process env
  or repository-local `.env`. It must not inspect parent Hesperides repos; this
  package stays standalone.
- Reuses one world-scoped ChatGPT conversation URL for text, stored at
  `agent_bridge/webgpt_conversation_binding.json`.
- Writes a redacted pending-turn prompt to `*-webgpt-prompt.md`.
- Builds an active memory revival packet with larger recent-event/memory
  windows, player-visible Archive View, query recall hits, and recent
  entity/relationship updates from world.db.
- Calls `webgpt_research` through the MCP wrapper.
- Reuses a per-world `agent_bridge/webgpt_conversation_binding.json` when
  WebGPT returns a conversation id.
- Extracts exactly one `AgentTurnResponse` JSON from `answer_markdown`.
- Commits that response through the same Rust validator as the Codex App
  backend.

`--webgpt-turn-command` remains available when an embedding host wants to
replace the built-in WebGPT MCP adapter with its own executable.

This keeps WebGPT as a swappable narrative engine, not a separate ChatGPT
conversation UI. Codex App remains the cost-balanced path: native thread
history plus a compact authoritative packet. WebGPT gets the heavier revival
packet because its project/session compaction behavior is less explicit.

Image generation is the same worker loop with a selectable visual backend. It
is not gated behind pending text-turn completion:

- Claim one pending visual job.
- For `--visual-backend codex-app-server`, ask Codex App app-server for exactly
  one `imageGeneration` and read the returned `savedPath`.
- For `--visual-backend webgpt`, call WebGPT MCP `webgpt_generate_image` and
  read the extracted generated-image PNG path. The image lane has its own
  world-scoped ChatGPT conversation URL at
  `agent_bridge/webgpt_image_conversation_binding.json`; prompts tell ChatGPT
  to treat previous images in that same URL as continuity references.
- Copy the PNG to `destination_path`.
- Complete the visual job and clear the claim.

When both text and image are `webgpt`, `host-worker` dispatches already-pending
text and visual work in parallel from the same tick so the text lane and image
lane consume their separate CDP sessions independently. If the text commit
creates a new turn-CG job during that tick, the worker immediately claims one
new visual job before exiting; it does not wait for a long-running keepalive
loop to come around again.

Turn CG has a major-character design gate. If a protagonist/anchor-level
character does not yet have an accepted character sheet under
`assets/vn/character_sheets/`, the scene prompt explicitly forbids direct
depiction of that character. It should use POV framing, environment-only
composition, off-screen presence, shadows, or cropped non-identifying fragments
until the sheet exists. Character sheet jobs themselves are still allowed; the
gate applies to scene CG exposure. WebGPT keeps turn CG and reference assets in
separate image conversations: turn CG uses
`agent_bridge/webgpt_image_conversation_binding.json`, while character/location
design assets use `agent_bridge/webgpt_reference_asset_conversation_binding.json`.
Reference assets are source material only and must not be displayed as scene CG.
Accepted reference assets listed on a turn-CG job are uploaded to WebGPT as
actual file attachments through `webgpt_generate_image.reference_paths`; a
local filesystem path written in the prompt is not considered visual evidence.

When the VN server also runs with `SINGULARI_WORLD_VISUAL_BACKEND=webgpt`, turn
CG cadence is more eager for the WebGPT image backend. The default cadence is 2
turns and can be overridden with `SINGULARI_WORLD_WEBGPT_TURN_CG_CADENCE_MIN`.

World-created backend choices take precedence over process flags. The launcher
writes `agent_bridge/backend_selection.json` with one text backend and one
visual backend, and marks it locked. Every tick, `host-worker` reads that
world-scoped selection before dispatching text or image work. `vn-serve` reads
the same selection before applying WebGPT's eager turn-CG cadence. The process
flags and `SINGULARI_WORLD_VISUAL_BACKEND` remain defaults only for older worlds
that have no locked selection file.

For text-only engine validation, pass `--visual-backend none`. The worker then
does not claim visual jobs, does not start a Codex App app-server only for CG,
and leaves pending CG jobs in the world store for a later visual worker.

Turn CG retry is allowed even when the current turn PNG already exists. A retry
marker creates a new `turn_cg:<turn_id>` job, runtime status reports it as
pending/claimed, and completion replaces the PNG and removes the retry marker.

The simulator binary never calls external image providers, `~/.codex` skills,
the active chat visual session, shell drawing scripts, SVG placeholders, or
local provider keys.

## Process Supervision

Use the release binary for normal play sessions:

```bash
cargo build --locked --release --bin singulari-world --bin singulari-world-mcp
target/release/singulari-world --store-root .world-store vn-serve --port 4177
```

`launchctl` is not part of the default deployment path. If a custom macOS host
still chooses to supervise a long-running worker with `launchctl`, do not rely
on the interactive shell PATH. Pass `--codex-bin /absolute/path/to/codex` and
set PATH so `/usr/bin/env node` can resolve `node`; the npm Codex launcher
depends on that lookup. A suitable launchd environment includes the directories
containing `node`, `codex`, and the system tools:

```text
PATH=/usr/local/bin:$HOME/.npm/bin:/usr/bin:/bin:/usr/sbin:/sbin
```

When replacing an old runtime, stop the old `vn-serve` and any managed
`codex app-server --listen ws://127.0.0.1:<port>` child before starting the new
server. Stale `dispatching` records are durable by design; if a one-shot worker
dies before writing a terminal dispatch record, remove only that failed dispatch
record before retrying the same pending turn.

## Reference CLI

Start a one-shot worker for the active or explicit world:

```bash
singulari-world host-worker --world-id <world-id> --once
```

Run an optional long-lived app worker:

```bash
singulari-world --store-root .world-store host-worker --interval-ms 750
```

Run the normal long-lived worker with WebGPT as the text engine:

```bash
singulari-world --store-root .world-store host-worker \
  --text-backend webgpt \
  --visual-backend webgpt \
  --interval-ms 750
```

The built-in WebGPT text and image lanes are not tab-switches inside one browser
session. `host-worker` launches them through separate MCP/browser sessions:
text defaults to CDP port `9238` and profile
`~/.hesperides/singulari-world/webgpt/text-profile`, while image defaults to
CDP port `9239` and profile
`~/.hesperides/singulari-world/webgpt/image-profile`. Override with
`SINGULARI_WORLD_WEBGPT_TEXT_CDP_PORT`,
`SINGULARI_WORLD_WEBGPT_IMAGE_CDP_PORT`,
`SINGULARI_WORLD_WEBGPT_TEXT_PROFILE_DIR`, and
`SINGULARI_WORLD_WEBGPT_IMAGE_PROFILE_DIR`. Startup fails if the two lanes share
the same port or profile. This prevents text generation, image generation, and
their world-scoped ChatGPT URLs from contaminating each other or blocking on one
browser queue.

On macOS, the CDP helper starts those Chrome windows minimized and reaps stale
matching Chrome processes when the window closes but the port/session state is
left behind. Windows browser lifecycle handling is not part of the current
operator path.

Resume an existing Codex App thread:

```bash
singulari-world --store-root .world-store host-worker \
  --world-id <world-id> \
  --codex-thread-id <codex-thread-id> \
  --interval-ms 750
```

Pass `--codex-app-server-url ws://127.0.0.1:<port>` only when the embedding host
already owns the app-server process. Pass `--codex-bin <path>` only when the
worker should start a managed app-server from a non-default Codex binary.

Omit `--world-id` in an embedding app. The worker waits until the browser
creates or loads a world, then follows the active world binding.

## JSONL Events

Every host worker event is one JSON object per line with schema version
`singulari.host_worker_event.v1`.

Startup:

```json
{"event":"worker_started","world_id":"stw_example","text_backend":"webgpt"}
```

Waiting for a world:

```json
{"event":"worker_waiting_for_active_world","text_backend":"codex-app-server"}
```

Realtime text dispatch:

```json
{"event":"codex_app_server_dispatch_started","world_id":"stw_example","turn_id":"turn_0001","thread_id":"019d...","turn_status":"completed"}
```

WebGPT text dispatch:

```json
{"event":"webgpt_dispatch_started","world_id":"stw_example","turn_id":"turn_0001","turn_status":"completed","response_path":"/path/to/turn_0001-webgpt-agent-response.json"}
```

Image job completed:

```json
{"event":"codex_app_image_generate_completed","world_id":"stw_example","slot":"turn_cg:turn_0001","status":"completed","saved_path":"/path/to/generated.png"}
```

WebGPT image job completed:

```json
{"event":"webgpt_image_generate_completed","world_id":"stw_example","slot":"turn_cg:turn_0001","status":"completed","generated_path":"/path/to/generated.png"}
```

Image job failed:

```json
{"event":"codex_app_image_generate_failed","world_id":"stw_example","slot":"turn_cg:turn_0001","status":"failed"}
```

Idle:

```json
{"event":"worker_idle","world_id":"stw_example","text_backend":"codex-app-server"}
```

## Failure Rules

- Hidden adjudication context never appears in host worker image prompts or VN
  packets.
- Missing or unstartable Codex app-server is fatal for Codex text dispatch and
  for visual-job dispatch.
- Missing WebGPT MCP wrapper or unusable WebGPT session is fatal for WebGPT text
  or visual dispatch. The worker does not silently fall back to deterministic
  script output.
- `--visual-backend none` must only skip visual work; it must not complete,
  delete, or mutate queued visual jobs.
- Visual jobs close only through the selected visual backend and the returned
  saved file: Codex App `imageGeneration`/`savedPath` or WebGPT
  `webgpt_generate_image`/extracted PNG path.
- Dispatch records and visual claims are durable world-store files, so worker
  restarts do not duplicate already-started jobs.
