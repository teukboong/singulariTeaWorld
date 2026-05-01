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
- `target/release/singulari-world-mcp-web`

## Codex MCP Install

Use the helper:

```bash
scripts/install-codex-mcp.sh
```

Direct install command:

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

## ChatGPT Web MCP

ChatGPT web developer mode expects a remote HTTPS MCP URL. The repository
provides a Streamable HTTP server for that host shape:

```bash
cargo build --locked --release --bin singulari-world-mcp-web
target/release/singulari-world-mcp-web \
  --host 127.0.0.1 \
  --port 4187 \
  --path /mcp \
  --profile play
```

Use a trusted HTTPS tunnel or reverse proxy in front of the local listener, then
configure ChatGPT with the public `/mcp` URL. The default `play` profile exposes
player-visible reads, player input submission, current CG image output, and an
image-ingest probe, plus `worldsim_complete_visual_job_from_base64` for
host-provided PNG payloads and `worldsim_complete_visual_job_from_url` for
HTTPS `image/png` URLs. It deliberately withholds hidden pending-turn context,
direct commit, repair, and generic visual-job completion from local paths.
`--profile authoring` is the bounded WebGPT text-backend connector profile: it
exposes `worldsim_next_turn_form` and `worldsim_submit_turn_form` plus
player-visible read/search tools, but not hidden pending-turn packets, direct
`worldsim_commit_agent_turn`, player input submission, DB repair, or local-path
visual completion. Use this profile for the WebGPT turn authoring connector.
`--profile read-only` removes player input submission and image completion.
`--profile trusted-local` is for private operator-controlled surfaces only.

ChatGPT Apps SDK VN component experiments are legacy. The supported play UI is
the shared `vn-serve` browser app. ChatGPT/WebGPT integration should attach as
a backend engine path or MCP backend contract, not as a second chat-embedded VN
client.

Image generation remains host-owned. `worldsim_current_cg_image` can return an
already saved PNG as MCP image content. `worldsim_probe_image_ingest` is the
compatibility probe for ChatGPT/App hosts that may be able to pass generated
image references back; it records only reference shape and byte counts, not the
image payload. If the host can pass PNG bytes, use
`worldsim_complete_visual_job_from_base64` with raw base64 or a
`data:image/png;base64,...` URL. If the host can pass a temporary URL, use
`worldsim_complete_visual_job_from_url`; the server accepts only HTTPS
`image/png`, rejects local/private hosts, private DNS resolution targets, and
credentials, follows at most three redirects, and caps the body at 16 MiB.

For WebGPT text/image backend activation behind the shared VN frontend, use
[webgpt-mcp-activation.md](webgpt-mcp-activation.md).

### Stable Cloudflare Front Door

For ChatGPT web, a stable Workers.dev URL can front the rotating free
`cloudflared` quick-tunnel URL:

```text
ChatGPT -> https://<worker>.workers.dev/mcp
        -> Worker KV origin=https://xxxx.trycloudflare.com
        -> cloudflared -> http://127.0.0.1:4187/mcp
```

Deploy the Worker in `cloudflare/worker/`:

```bash
SINGULARI_WORLD_CF_KV_NAMESPACE_ID=<kv namespace id>
SINGULARI_WORLD_FRONTDOOR_UPDATE_SECRET=<same secret>
SINGULARI_WORLD_FRONTDOOR_URL=https://<worker>.workers.dev

scripts/deploy_cloudflare_frontdoor.sh
```

Then configure the local repository `.env`:

```bash
SINGULARI_WORLD_FRONTDOOR_URL=https://<worker>.workers.dev
SINGULARI_WORLD_FRONTDOOR_UPDATE_SECRET=<same secret>
```

Run the MCP listener and tunnel in separate terminals:

```bash
cargo run --locked --bin singulari-world-mcp-web -- \
  --host 127.0.0.1 \
  --port 4187 \
  --path /mcp \
  --profile play

scripts/run_mcp_tunnel.sh
```

The tunnel script writes the last synced quick-tunnel origin to
`.runtime/mcp_tunnel_base_url.txt` and retries pending Worker origin updates
from `.runtime/mcp_tunnel_origin_pending.txt`.

This is a separate Singulari deployment. Reuse the Railbot pattern, not the
Railbot Worker, KV namespace, or secrets.

For the full setup and troubleshooting guide, use
[cloudflare-free-frontdoor.md](cloudflare-free-frontdoor.md).

## Local VN Runtime

Create a world:

```bash
cargo run --locked --bin singulari-world -- start \
  --seed-text "medieval border village, young patrol, sealed road marker" \
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

For Tailscale phone play, keep the same responsive web app and bind only to a
Tailscale address or hostname:

```bash
cargo run --locked --bin singulari-world -- vn-serve \
  --host <tailscale-ip-or-hostname> \
  --port 4177 \
  --trusted-tailnet
```

The VN server is intentionally not a general LAN server. Default exposure is
loopback-only. `--trusted-tailnet` explicitly opts into Tailscale phone play and
assumes tailnet peers can read `/app.js` and recover the CSRF token; `0.0.0.0`
and normal LAN addresses should fail closed.

## Background Worker Contract

The browser writes durable pending jobs. It does not call an LLM directly.
WebGPT is the only runtime backend:

```bash
target/release/singulari-world --store-root .world-store host-worker \
  --text-backend webgpt \
  --visual-backend webgpt \
  --webgpt-output-mode tool-form \
  --interval-ms 750
```

The worker sends connector-use instructions through this repository's bundled
`webgpt-mcp-checkout/scripts/webgpt-mcp.sh` by default. Explicit wrapper
overrides must still point inside this repository; sibling or parent checkouts
are rejected so the public-alpha package stays standalone. In `tool-form` mode,
WebGPT must call the Singulari World MCP connector tools
`worldsim_next_turn_form` and `worldsim_submit_turn_form`; the host-worker no
longer parses TurnFormSubmission JSON from assistant text. The durable commit
record produced by `worldsim_submit_turn_form` is the success source, so the UI,
DB, CG queue, and redaction rules stay shared. With `--visual-backend
webgpt`, it also calls
`webgpt_generate_image`, receives an extracted PNG path, and completes the same
visual jobs the browser queued. WebGPT text and image use separate world-scoped
ChatGPT conversation URLs: `agent_bridge/webgpt_conversation_binding.json` for
text and `agent_bridge/webgpt_image_conversation_binding.json` for images.
They also use separate WebGPT browser sessions from process start: text owns CDP
port `9238`, turn CG image owns `9239`, and reference-asset image owns `9240`,
each with its own profile dir.
Configure the `SINGULARI_WORLD_WEBGPT_*_CDP_PORT` and
`SINGULARI_WORLD_WEBGPT_*_PROFILE_DIR` variables only if those defaults collide
with another local service.
Worlds created through the VN launcher also store a locked backend pair at
`agent_bridge/backend_selection.json`. The valid pair is WebGPT/WebGPT, and old
local Codex App selections must not revive a retired backend. The WebGPT cadence
defaults to 2 turns and can be overridden with
`SINGULARI_WORLD_WEBGPT_TURN_CG_CADENCE_MIN`. Use `--visual-backend none` only
for text-only smoke tests.

Automatic image jobs can also go through the installed `singulari-world-mcp`
server. `worldsim_claim_visual_job` returns structured content containing
`job.image_generation_call`; the image host generates from that call and then
calls `worldsim_complete_visual_job`. The host worker uses the same
claim/complete contract.

## Current Alpha Boundary

The standalone simulator owns:

- world persistence
- VN projection
- MCP tools
- ChatGPT web MCP Streamable HTTP adapter
- text-turn pending/commit
- WebGPT conversation bindings
- host-worker event supervisor
- image job WebGPT and MCP completion contracts

The embedding host still owns:

- keeping WebGPT browser sessions authenticated
- starting/stopping `host-worker`
- starting/stopping the VN server
- consuming or supervising visual jobs through the selected backend, then
  completing or releasing the claim
