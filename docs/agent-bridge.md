# Agent Bridge

`singulari-world` separates durable world state from narrative authorship.

The normal flow is:

```text
player input
  -> pending turn
  -> Codex App host-worker authors visible scene
  -> server validates and commits
  -> VN packet renders the new scene
```

The browser only receives player-visible packets. Hidden adjudication context is
available through the MCP `worldsim_next_pending_turn` tool so a trusted local
agent can judge plausibility without leaking secrets to the player surface.

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

ChatGPT web uses the separate `singulari-world-mcp-web` binary over Streamable
HTTP. Its default `play` profile is intentionally narrower than the local stdio
MCP: it can read player-visible state, submit player input, return an existing
CG as MCP image content, record image-ingest probe shapes, and complete a visual
job from host-provided PNG base64 or an HTTPS `image/png` URL. It cannot read
hidden pending-turn adjudication or directly commit agent-authored turns unless
started with `--profile trusted-local`.

`worldsim_commit_agent_turn` rejects visible text that directly includes hidden
truth strings or forbidden leak strings from the pending packet.

## Codex App Background Worker

The VN browser does not call an LLM by itself. It writes durable pending jobs
into the world store. `host-worker` is the app-facing process that closes those
jobs through Codex App app-server.

For local Codex App play, the operator phrase `싱귤러리 월드 준비해줘` means:

```bash
target/release/singulari-world --store-root .world-store host-worker \
  --interval-ms 750
```

Codex App should remain open while the VN browser is used. The worker starts a
managed loopback `codex app-server` when no `--codex-app-server-url` is passed,
dispatches pending text turns through that websocket, and commits completed text
results back into the world store. The same worker consumes visual jobs through
Codex App `imageGeneration` and writes completion metadata. When there is no
active world or no pending work, it idles.

For a packaged app, Codex App should own this cross-platform background process
instead of relying on OS-specific schedulers. Start it before opening the VN
app, stop it when the app closes. Omit `--world-id` in the normal app flow so it
can follow whichever world the browser creates or loads.

The worker prints newline-delimited JSON events for dispatch and completion
state while also closing the work itself:

```json
{"event":"codex_app_server_dispatch_started","world_id":"...","turn_id":"turn_0001","turn_status":"completed"}
{"event":"codex_app_image_generate_completed","world_id":"...","slot":"turn_cg:turn_0001","status":"completed"}
```

Image generation is queue-based: the simulator exposes a redacted visual job,
then `host-worker` claims it, asks Codex App app-server for exactly one
`imageGeneration`, copies the returned saved PNG, writes completion metadata,
and clears the active claim.

## Thread Binding

Each world may have a durable Codex App thread binding:

```bash
singulari-world --store-root .world-store codex-thread-bind \
  --world-id <world-id> \
  --thread-id <codex-thread-id>

singulari-world --store-root .world-store codex-thread-show --world-id <world-id>
```

Passing `--codex-thread-id` to `host-worker` seeds or refreshes that binding for
the watched world:

```bash
singulari-world --store-root .world-store host-worker \
  --world-id <world-id> \
  --codex-thread-id <codex-thread-id> \
  --interval-ms 750
```

The world-specific Codex thread is the narrative working context, not the source
of truth. By default, `host-worker` runs with
`--codex-thread-context-mode native-thread`: resumed app-server turns include the
Codex App thread history for prose rhythm and immediate scene continuity, while
each turn injects only a compact authoritative world packet for current state,
hidden adjudication, and output contract. If a bound thread cannot be resumed
because it is stale or missing, the worker clears that world's binding so the
next dispatch can start a fresh thread from the same world DB.

Use `--codex-thread-context-mode bounded-packet` when the thread history should
be excluded and the full pending world-store packet should be reinjected every
turn. This is more deterministic but grows the per-turn prompt faster.

If no `--codex-app-server-url` is provided, `host-worker` starts
`codex app-server` on a loopback port and writes
`codex_app_server_runtime.json` under the store-root `agent_bridge` directory.
Pass an explicit URL only when the embedding host owns the app-server process.

After the background worker is ready, the browser-facing runtime is just the VN
server:

```bash
target/release/singulari-world --store-root .world-store vn-serve --port 4177
```

For Tailscale phone play, serve the same app on a Tailscale address or hostname:

```bash
target/release/singulari-world --store-root .world-store vn-serve \
  --host <tailscale-ip-or-hostname> \
  --port 4177
```

Do not use a regular LAN bind or `0.0.0.0`; the VN server allowlist is loopback
plus Tailscale.

No external image API is part of the runtime contract. The standalone MCP and
host-worker own job discovery and redacted prompts; Codex App owns actual image
generation and filesystem save.
