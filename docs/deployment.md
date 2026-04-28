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
host-provided PNG payloads. It deliberately withholds hidden pending-turn
context, direct commit, repair, and generic visual-job completion from local
paths. `--profile read-only` removes player input submission and base64
completion. `--profile trusted-local` is for private operator-controlled
surfaces only.

Image generation remains host-owned. `worldsim_current_cg_image` can return an
already saved PNG as MCP image content. `worldsim_probe_image_ingest` is the
compatibility probe for ChatGPT/App hosts that may be able to pass generated
image references back; it records only reference shape and byte counts, not the
image payload. If the host can pass PNG bytes, use
`worldsim_complete_visual_job_from_base64` with raw base64 or a
`data:image/png;base64,...` URL.

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

For Tailscale phone play, keep the same responsive web app and bind only to a
Tailscale address or hostname:

```bash
cargo run --locked --bin singulari-world -- vn-serve \
  --host <tailscale-ip-or-hostname> \
  --port 4177
```

The VN server is intentionally not a general LAN server. Loopback and Tailscale
are allowed; `0.0.0.0` and normal LAN addresses should fail closed.

## Background Worker Contract

The browser writes durable pending jobs. It does not call an LLM directly.

Codex App prep flow:

```bash
target/release/singulari-world --store-root .world-store host-worker \
  --interval-ms 750
```

This is what the Codex App agent should start when the operator says
`싱귤러리 월드 준비해줘`. By default, the worker starts `codex app-server` on
a managed loopback port, records the runtime URL in the store-root
`agent_bridge` directory, and dispatches only when a pending world turn exists.
It can be started before any world exists; it idles until the browser creates or
loads the active world. Visual jobs close through the same app-server loop:
claim -> Codex App `imageGeneration` -> saved PNG -> completion metadata.
Keep Codex App open while playing. Hosts that already own the websocket may pass
`--codex-app-server-url`.

Text turns default to `--codex-thread-context-mode native-thread`: Codex App
thread history carries narrative rhythm, and each dispatch injects only a
compact authoritative world packet for current state. Use `bounded-packet` for
the older full-packet reinjection mode.

For macOS `launchctl` supervision, use absolute paths. If `codex` comes from npm,
the launcher uses `/usr/bin/env node`; set PATH in the LaunchAgent so `node` is
visible, or managed app-server startup will fail before listening.

Automatic image jobs go through the installed `singulari-world-mcp` server.
`worldsim_claim_visual_job` returns structured content containing
`job.codex_app_call`; Codex App consumes that call with its built-in image
generation capability and then calls `worldsim_complete_visual_job`.

## Current Alpha Boundary

The standalone simulator owns:

- world persistence
- VN projection
- MCP tools
- ChatGPT web MCP Streamable HTTP adapter
- text-turn pending/commit
- Codex thread binding
- host-worker event supervisor
- image job app-server and MCP completion contracts

The embedding host still owns:

- keeping Codex App open and authenticated
- starting/stopping `host-worker`
- optionally passing `--codex-app-server-url` when it owns app-server itself
- starting/stopping the VN server
- consuming visual jobs through the standalone MCP `codex_app_call` contract,
  then completing or releasing the claim
