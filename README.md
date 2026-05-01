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

Detailed operator guides:

- [WebGPT MCP Activation Guide](docs/webgpt-mcp-activation.md)
- [Cloudflare Free Front Door Guide](docs/cloudflare-free-frontdoor.md)
- [Architecture V2](docs/architecture-v2.md)
- [Host Worker Contract](docs/host-worker.md)
- [Agent Bridge](docs/agent-bridge.md)
- [VN Engine Player Surface Blueprint](docs/vn-engine-player-surface-blueprint.md)
- [Visual Canon and Budget Policy](docs/visual-canon-budget.md)
- [Causal Simulation Upgrade Set](docs/causal-simulation-upgrade-set.md)

## Quick Start

```bash
scripts/smoke-local.sh
scripts/setup-webgpt-runtime.sh

cargo run --locked --bin singulari-world -- start \
  --seed-text "<world-seed>" \
  --json

cargo run --locked --bin singulari-world -- vn-serve --port 4177
```

For public-alpha release confidence, run the committed-state clone gate:

```bash
scripts/fresh-clone-e2e.sh
```

`scripts/setup-webgpt-runtime.sh` is the one-time WebGPT backend prep for a
fresh clone. It installs the bundled `chatgpt-worker` npm dependencies, rebuilds
the worker bundle, and builds `webgpt-mcp-checkout/target/release/webgpt-mcp`.
After that, the default host worker uses this repo's bundled
`webgpt-mcp-checkout/scripts/webgpt-mcp.sh` without needing a parent
Hesperides checkout.

## Common VN Web Play Mode

The VN browser app is the common play frontend. WebGPT is the only text/image
backend; changing engines must not create a second chat-native UI.

Prep command:

```bash
cargo build --locked --release --bin singulari-world --bin singulari-world-mcp
target/release/singulari-world --store-root .world-store vn-serve --port 4177
```

Open:

```text
http://127.0.0.1:4177/
```

`vn-serve` is the normal play supervisor. It starts one resident `host-worker`
for the active store, and browser-created pending turns or CG retry requests are
picked up by that worker instead of spawning a fresh worker per click. The
worker consumes pending text and visual jobs, writes the visible result back to
the world store, and keeps its WebGPT text-lane MCP session warm across turns.
No `launchd`/KeepAlive setup is required for the default deployment UX.

The worker owns visual jobs through the WebGPT image loop. It claims one pending
visual job per tick, calls WebGPT MCP `webgpt_generate_image`, extracts the
generated ChatGPT image as a PNG, writes it to `destination_path`, and completes
the job. It never uses local placeholders, shell drawing scripts, `~/.codex`
skills, or the active chat visual session. Image-lane failures are emitted as
worker events and released for retry; they must not kill the resident text
worker or delay the next player input. For explicit text-only testing, pass
`--visual-backend none` so CG jobs remain queued. Visual jobs are not gated
behind pending text-turn completion; text and image dispatch run in parallel
against their separate browser sessions.

For operator diagnostics, the equivalent worker command is:

```bash
target/release/singulari-world --store-root .world-store host-worker \
  --text-backend webgpt \
  --visual-backend webgpt \
  --webgpt-output-mode tool-form \
  --interval-ms 750
```

By default the worker uses the bundled
`webgpt-mcp-checkout/scripts/webgpt-mcp.sh` wrapper. It may use
`SINGULARI_WORLD_WEBGPT_MCP_WRAPPER` in the process env or repository-local
`.env` / `--webgpt-mcp-wrapper` only when that path still points inside this
repository. It does not inspect sibling or parent checkouts; the public-alpha
package must stay standalone. In `tool-form` mode it sends a connector-use
instruction through `webgpt_research`; WebGPT must call the Singulari World MCP
tools `worldsim_next_turn_form` and `worldsim_submit_turn_form`. The host-worker
does not parse TurnFormSubmission JSON from assistant text. It records the
assistant text as operator evidence and treats the durable
`agent_bridge/committed_turns/<turn_id>/commit_record.json` created by
`worldsim_submit_turn_form` as the success source. `draft` and
`agent-response` modes remain fallback/diagnostic paths.
The WebGPT visual backend calls `webgpt_generate_image`, receives a saved PNG
path from the MCP worker, and completes the queued visual job through the same
Rust store contract. Each world gets separate persistent ChatGPT conversation
URLs for WebGPT text, turn CG, and reference-asset work: the text lane stores
`agent_bridge/webgpt_conversation_binding.json`, turn CG stores
`agent_bridge/webgpt_image_conversation_binding.json`, and character/location
design assets store `agent_bridge/webgpt_reference_asset_conversation_binding.json`.
Reference assets are source material only; they must not share the turn-CG
conversation or appear as scene CG. When a turn-CG job lists accepted reference
asset paths, WebGPT uploads those PNG/JPEG/WebP/GIF files as attachments in the
same image-generation message; local path notes are not treated as a substitute
for attachment. These lanes also run in separate browser sessions, not by
switching tabs in one worker: text defaults to CDP port `9238`, turn CG image
defaults to `9239`, and reference-asset image defaults to `9240`, with separate
profile roots under `~/.hesperides/singulari-world/webgpt/`. Resident
`host-worker` processes prewarm the lanes once so the three-way split is
visible and port/profile collisions fail early. Browser play reuses that
resident worker; the text lane also reuses a resident MCP stdio client instead
of spawning `webgpt-mcp client-call` for every turn. The worker can
claim one turn-CG job and one reference-asset job in the same tick, so design
asset generation no longer blocks scene CG. New worlds created from the VN
launcher write a locked `agent_bridge/backend_selection.json`; the valid
backend pair is WebGPT/WebGPT. Old local `codex-app-server` selections are
legacy data and must not start Codex App runtime plumbing.

Text prompts include an explicit `allowed_reference_atoms` list compiled from
the narrative turn packet. WebGPT must copy refs from that list for
`gate_ref`, `evidence_refs`, `target_refs`, and `grounding_ref`; free-text refs
such as invented `mind:*` labels are audit failures, not a repair path to rely
on during normal play.

Inspect the exact deterministic continuity packet sent to WebGPT text without
launching a browser:

```bash
singulari-world revival-packet --world-id <world-id> --json
```

WebGPT turn CG cadence defaults to every 2 turns; override it with
`SINGULARI_WORLD_WEBGPT_TURN_CG_CADENCE_MIN`. `--webgpt-turn-command` remains
available for a custom text adapter.

Turn CG does not directly expose major characters before their designs are
accepted. Until the protagonist or anchor-level character has a completed
`assets/vn/character_sheets/*.png`, the prompt forces POV/environment/off-screen
framing instead of faces, full-body front views, distinctive outfits, or clear
identifiable silhouettes.

For automatic image completion, the installed `singulari-world-mcp` server is
the standalone bridge. `worldsim_claim_visual_job` returns structured MCP
content with `job.image_generation_call`; the image host generates from that
call, writes the PNG to `destination_path`, then calls
`worldsim_complete_visual_job`.

Each world owns durable WebGPT conversation URL bindings under `agent_bridge/`.
The world DB remains source of truth, so ChatGPT session loss or compaction does
not erase canon. When no active world exists yet, `host-worker` idles until the
browser creates or loads one.

For phone play over Tailscale, use the same VN web app, not a separate mobile
URL:

```bash
target/release/singulari-world --store-root .world-store vn-serve \
  --host <tailscale-ip-or-hostname> \
  --port 4177 \
  --trusted-tailnet
```

The default VN exposure mode is loopback-only. `--trusted-tailnet` is required
for Tailscale binds because the injected `X-Singulari-VN-Token` only protects
against blind CSRF; any peer that can fetch `/app.js` can read that token. Do
not use `0.0.0.0` or regular LAN addresses as a shortcut.

The local Codex MCP server runs over stdio:

```bash
cargo run --locked --bin singulari-world-mcp
```

Install it into Codex:

```bash
scripts/install-codex-mcp.sh
```

The MCP surface includes `worldsim_visual_assets`, which returns the same
player-visible manifest and host image generation jobs without requiring a
separate provider key in this repo. `worldsim_claim_visual_job` also includes
the current turn CG job from the VN packet when it is pending. The WebGPT image
host claims and completes those jobs through `worldsim_claim_visual_job` and
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
the host as MCP image content through `worldsim_current_cg_image`. Use
`--profile authoring` for the WebGPT text backend connector; it exposes
`worldsim_next_turn_form` and `worldsim_submit_turn_form` plus player-visible
reads without exposing hidden pending-turn packets, direct trusted-local commit,
repair, or local-path visual completion.
The probe tool records which image reference shapes the host can pass back
(`image_base64`, `image_url`, `resource_uri`, or `file_id`) without persisting
image bytes. If the host can pass PNG bytes, complete the pending visual job
with `worldsim_complete_visual_job_from_base64` using raw base64 or a
`data:image/png;base64,...` URL. If it can pass a temporary URL, use
`worldsim_complete_visual_job_from_url`; the server accepts only HTTPS
`image/png`, rejects local/private hosts, private DNS resolution targets, and
credentials, limits redirects, and caps the download size.

ChatGPT chat-native UI experiments are legacy. The supported play surface is
the shared `vn-serve` browser UI; ChatGPT/WebGPT integration should plug into
the worker engine slot or the MCP backend contract, not into a separate
conversation widget.

For the full WebGPT text/image setup sequence, see
[WebGPT MCP Activation Guide](docs/webgpt-mcp-activation.md).

### Stable Cloudflare Front Door

For a free stable URL, use the same front-door pattern as Railbot:

1. Deploy `cloudflare/worker/` to Workers.dev with a Workers KV namespace.
2. Set Worker secret `ORIGIN_UPDATE_SECRET`.
3. Put the fixed Worker URL and the same secret in local `.env`:

```bash
SINGULARI_WORLD_FRONTDOOR_URL=https://<worker>.workers.dev
SINGULARI_WORLD_FRONTDOOR_UPDATE_SECRET=<same secret>
```

4. Run the local MCP server and tunnel:

```bash
cargo run --locked --bin singulari-world-mcp-web -- \
  --host 127.0.0.1 \
  --port 4187 \
  --path /mcp \
  --profile play

scripts/run_mcp_tunnel.sh
```

The ChatGPT custom app MCP URL is:

```text
https://<worker>.workers.dev/mcp
```

`scripts/run_mcp_tunnel.sh` watches the free `cloudflared` quick-tunnel URL and
updates Worker KV through `/_singulari/origin`, so ChatGPT keeps using the
stable Worker URL.

For the full zero-cost Workers.dev + quick tunnel setup, see
[Cloudflare Free Front Door Guide](docs/cloudflare-free-frontdoor.md).

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

## Diagnostics and Repair

```bash
singulari-world validate --world-id <world-id> --json
singulari-world projection-health --world-id <world-id> --json
singulari-world repair-turn-materializations --world-id <world-id> --json
singulari-world recover-turn-commit-journal --world-id <world-id> --json
singulari-world repair-db --world-id <world-id> --json
singulari-world repair-extra-memory --world-id <world-id> --json
singulari-world visual-job-release --world-id <world-id> --slot <slot> --json
```

`projection-health` is the first diagnostic. It checks world files, world.db,
turn commit journal/materializations, extra memory projection records, and the
unified job ledger. Use repair commands only for the component it names; no
hidden repair runs during normal play. `recover-turn-commit-journal` is the
crash-recovery path for prepared/failed turn envelopes that already have durable
commit records.

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
