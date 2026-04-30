# Singulari World Agent Guide

This repository is a standalone, public-safe text world simulator with a local
VN web projection and MCP tools. Start here after cloning.

## Boundaries

- Fallback and defensive-code detours are forbidden as fixes. Close the real
  loop: identify the authoritative producer/consumer contract, fix that path,
  and verify the same loop end to end.
- Do not commit local world stores, generated images, DB files, or private
  narrator/world presets.
- Runtime state belongs in `.world-store/`, `SINGULARI_WORLD_HOME`, or an
  explicit export chosen by the user.
- Browser-visible packets must stay player-visible only. Hidden adjudication
  context is for trusted local agents and must not leak into visible prose,
  Codex View, image prompts, or logs.
- The simulator does not call external image APIs directly. It emits redacted
  image jobs; the selected host backend owns image generation and PNG save.

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

Before tagging a public-alpha cut, also prove the committed checkout from a
clean clone:

```bash
scripts/fresh-clone-e2e.sh
```

## CLI Basics

Create a world:

```bash
cargo run --locked --bin singulari-world -- start \
  --seed-text "medieval border village, young patrol, sealed road marker" \
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

## WebGPT Runtime Prep

When the operator says `싱귤러리 월드 준비해줘`, prepare the VN runtime and the
WebGPT-only backend, not a one-off chat turn.

From this repository, build the binary and start the VN server:

```bash
cargo build --locked --release --bin singulari-world --bin singulari-world-mcp
scripts/setup-webgpt-runtime.sh

target/release/singulari-world --store-root .world-store vn-serve --port 4177
```

`vn-serve` owns the default deployment UX. It starts one resident `host-worker`
for the active store. When the browser creates a pending turn, asks for CG
retry, or submits work, that resident worker consumes pending text and visual
jobs through WebGPT and writes validated results back to the store. Do not
require `launchd`/KeepAlive for normal play. Visual jobs must close through the
repo-owned visual-job contract: claim -> WebGPT image generation -> PNG saved to
`destination_path` -> complete. Do not route them through this Codex chat's
visual generation session. Image-lane failures emit retryable worker events;
they must not kill the resident text worker or delay the next player input.

The VN browser app is the only player-facing frontend. WebGPT is the only
runtime backend. For diagnostics, the equivalent worker command is:

```bash
target/release/singulari-world --store-root .world-store host-worker \
  --text-backend webgpt \
  --visual-backend webgpt \
  --interval-ms 750
```

The built-in backend calls `webgpt_research` through the bundled
`webgpt-mcp-checkout/scripts/webgpt-mcp.sh` wrapper unless
`--webgpt-mcp-wrapper` or `SINGULARI_WORLD_WEBGPT_MCP_WRAPPER` in process env
or repository-local `.env` overrides it, and overrides must still point inside
this repository. It must not inspect parent Hesperides repos or sibling
checkouts; this package stays standalone. It extracts one `AgentTurnResponse` JSON
from `answer_markdown`; the Rust worker owns validation and commit. Do not add
a separate ChatGPT conversation UI as a second play client.
WebGPT uses an active memory revival packet: larger `resume_pack`,
player-visible Archive View, query recall hits, and recent entity/relationship
updates from world.db are surfaced before each turn.
The same worker calls `webgpt_generate_image`,
extracts the generated ChatGPT image to a PNG path, and completes the queued
visual job through the normal store contract. Use `--visual-backend none` only
for explicit text-only validation so CG jobs stay queued.
Each world has separate WebGPT URL bindings for text and image. Text uses
`agent_bridge/webgpt_conversation_binding.json`; image uses
`agent_bridge/webgpt_image_conversation_binding.json` and treats prior generated
images in that same ChatGPT conversation as visual continuity references.
WebGPT text and image lanes must run as separate browser sessions, not as one
window that switches tools. Text defaults to CDP port `9238`, turn CG image
defaults to `9239`, and reference/design image defaults to `9240`; the profile
dirs are separate under `~/.hesperides/singulari-world/webgpt/`. Long-running
`host-worker` processes prewarm all three lane sessions once before dispatch;
browser play reuses the resident worker and the text lane reuses a resident
MCP stdio client, so player input is not delayed by repeated worker and
`webgpt-mcp client-call` startup. Starting with a shared port or shared profile
is a contract violation.
The VN launcher also writes a locked `agent_bridge/backend_selection.json` on
world creation. The only valid text/visual backend is WebGPT. Old local
`codex-app-server` selections are legacy data and must not start Codex App
runtime plumbing.

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

ChatGPT Apps SDK VN panel experiments are legacy. The supported player-facing
UI is the shared `vn-serve` browser app; ChatGPT/WebGPT work should plug in as a
backend/engine path behind the worker or as MCP backend calls, not as a separate
chat-embedded client.

Use [docs/webgpt-mcp-activation.md](docs/webgpt-mcp-activation.md) when enabling
WebGPT text/image backends. It is the operator checklist for wrapper discovery,
separate text/image CDP sessions, world-scoped conversation bindings, backend
selection locks, and WebGPT image cadence.

Turn CG and reference assets are separate visual contracts. `turn_cg:*` jobs may
write scene PNGs under `assets/vn/turn_cg/`; character sheets, location sheets,
menu/stage backgrounds, and other design assets are source material only and
must stay in their asset paths. WebGPT turn CG uses
`agent_bridge/webgpt_image_conversation_binding.json`; reference assets use
`agent_bridge/webgpt_reference_asset_conversation_binding.json` so character
design sheets do not slide into scene CG through shared conversation history.
When turn CG has accepted `reference_paths`, WebGPT must receive those files as
actual image attachments through `webgpt_generate_image.reference_paths`.
Prompt-only local path notes are audit hints, not visual reference delivery.

Narrative defaults must not inject genre priors. Compact seeds like `중세
남자주인공` may define only title/genre/protagonist fragments; do not infer
modern reincarnation, isekai transfer, possession, regression, system windows,
cheat powers, hospitals, electricity, addresses, or other genre tropes unless
they are explicit in seed premise or player-visible canon.

World simulator V2 is pressure-first, not trope-first. Keep every turn lively by
moving at least one visible pressure vector: survival, social, material, threat,
mystery, desire, moral cost, or time pressure. Removing genre bias must not turn
the output into dry logs.

Slots 1-5 are scene-specific presented choices. Slot 6 is always inline
`자유서술`. Slot 7 is `판단 위임`: a meta-GM judgment slot, not an in-world
guide or hidden character. Keep its visible intent redacted as `맡긴다. 세부
내용은 선택 후 드러난다.` Legacy `안내자의 선택` rows may be read for old worlds,
but new output uses slot 7 `판단 위임`.

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

## Cloudflare Front Door

For ChatGPT web, keep the local MCP listener on loopback and expose it through
the Worker front door in `cloudflare/worker/`. The Worker gives ChatGPT a stable
HTTPS `/mcp` URL while `scripts/run_mcp_tunnel.sh` rotates the free
`cloudflared` quick-tunnel origin behind it.

Local secret/config values belong in `.env`, which is gitignored:

```bash
SINGULARI_WORLD_FRONTDOOR_URL=https://<worker>.workers.dev
SINGULARI_WORLD_FRONTDOOR_UPDATE_SECRET=<same secret as Worker ORIGIN_UPDATE_SECRET>
```

Do not commit Cloudflare tokens, Worker secrets, or local tunnel state.

Use [docs/cloudflare-free-frontdoor.md](docs/cloudflare-free-frontdoor.md) for
the complete Workers.dev + Workers KV + free `cloudflared` quick-tunnel setup.
This deployment is dedicated to Singulari World; do not reuse Railbot's Worker,
KV namespace, or secrets.

## Agent-Authored Text Turns

The browser queues player input; a trusted local agent commits the visible turn.

For normal play, prefer the prep command above. Manual `agent-submit` /
`agent-next` / `agent-commit` commands are debugging tools.

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

For realtime dispatch, run the WebGPT host worker:

```bash
singulari-world host-worker \
  --text-backend webgpt \
  --visual-backend webgpt \
  --interval-ms 750
```

`vn-serve` is the app-facing supervisor for default play. It starts a resident
`host-worker` for browser-created work instead of spawning a worker per click.
It never starts `codex app-server` and never reads Codex thread bindings. When
no active world exists, the resident worker waits for the user to create or load
one.

`launchctl` is not the normal deployment path. If a custom host starts a
long-running `host-worker`, it still must use the same WebGPT-only flags and
separate CDP ports.

`host-worker` calls `webgpt_research` through the configured WebGPT MCP wrapper
for text. The worker stores a
per-world `webgpt_conversation_binding.json`, extracts one `AgentTurnResponse`
JSON from the WebGPT answer, and commits it through the same world-store
validator, so hidden redaction and schema checks stay identical. WebGPT is not
trusted to remember canon from opaque project memory alone; host-worker
proactively injects DB-backed revival context every turn. Use
`--webgpt-turn-command` only to replace the built-in MCP adapter.
The visual lane uses separate per-world image conversation bindings
for ChatGPT image generation and closes visual jobs with the saved extracted
PNG. The built-in WebGPT lanes must use separate browser sessions: text
defaults to CDP port `9238`, turn CG image defaults to `9239`, reference/design
image defaults to `9240`, and each lane has its own profile dir under
`~/.hesperides/singulari-world/webgpt/`. Resident `host-worker` processes
prewarm all three lane sessions for WebGPT/WebGPT worlds before dispatch; the
text lane keeps a resident MCP stdio client alive so the actual text call can
attach to its lane without redoing worker and MCP startup every turn.
Text prompts also include `allowed_reference_atoms` compiled from the narrative
turn packet. WebGPT must copy refs from that list for `gate_ref`,
`evidence_refs`, `target_refs`, and `grounding_ref`; invented free-text refs
such as `mind:*` labels are audit failures, not normal repair budget.
Worlds created by the VN launcher write a locked
`agent_bridge/backend_selection.json`. That file records WebGPT/WebGPT and keeps
old backend flags from reintroducing a second engine. The default WebGPT cadence
is `SINGULARI_WORLD_WEBGPT_TURN_CG_CADENCE_MIN=2`. `--visual-backend none`
disables only visual claiming/generation for explicit text-only checks; pending
CG jobs remain in the store for a later visual worker.
Use `singulari-world revival-packet --world-id <world-id> --json` to inspect
the exact WebGPT text continuity packet. The packet is the source-of-truth
revival surface; ChatGPT project/session memory is not.

## Visual Job Worker

Image jobs are host-consumed jobs, not `codex exec` jobs. The same
`host-worker` started by `싱귤러리 월드 준비해줘` owns the visual loop: claim one
job, ask WebGPT MCP `webgpt_generate_image` for one generated image, copy/save
the PNG, then complete the job. The Codex chat/session-level `image_gen` path is
not an acceptable substitute.
Turn CG prompts must not directly expose major characters before their design
sheet exists. Until a protagonist/anchor-level character has an accepted
`assets/vn/character_sheets/*.png`, scene CG should use POV framing,
environment-only composition, off-screen presence, shadows, or cropped
non-identifying body fragments rather than faces, full-body front views,
distinctive outfits, or identifiable silhouettes.

Turn CG retry is a regeneration request. If a current turn image already exists,
the retry marker still creates a new `turn_cg:<turn_id>` job; completion
overwrites the turn PNG, clears the visual claim, and removes the retry marker.

The MCP path uses the standalone `singulari-world-mcp` tool surface.
`worldsim_claim_visual_job` returns structured MCP content containing
`job.image_generation_call`. The image host generates from that call, writes the
PNG to `destination_path`, then calls `worldsim_complete_visual_job`. This is
the repo-owned MCP contract; it does not depend on `~/.codex` skills, external
provider keys, or the active chat visual session.

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
singulari-world projection-health --world-id <world-id> --json
singulari-world repair-turn-materializations --world-id <world-id> --json
singulari-world recover-turn-commit-journal --world-id <world-id> --json
singulari-world repair-db --world-id <world-id> --json
singulari-world repair-extra-memory --world-id <world-id> --json
singulari-world visual-job-release --world-id <world-id> --slot <slot> --json
```

Run `projection-health` before repair. It checks world files, world.db, turn
commit journal/materializations, extra memory projections, and the unified job
ledger. Repair only the failed component it names; no fallback or hidden
auto-repair during normal play. Use `recover-turn-commit-journal` when a crash
or worker restart left prepared/failed turn envelopes but durable commit records
already exist.

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
- WebGPT text dispatch
- host worker supervisor contract
- visual job WebGPT and MCP completion contracts
- privacy audit gate for tracked files and git history
- release and smoke scripts

Still host-owned:

- ChatGPT/WebGPT login state for the browser profiles
- app-managed start/stop of `host-worker`
- packaged installers for macOS/Windows/Linux
