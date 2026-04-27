# Agent Bridge

`singulari-world` separates durable world state from narrative authorship.

The normal flow is:

```text
player input
  -> pending turn
  -> trusted local agent writes visible scene and structured updates
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
- `worldsim_claim_visual_job`
- `worldsim_complete_visual_job`
- `worldsim_release_visual_job`
- `worldsim_resume_pack`
- `worldsim_search`
- `worldsim_codex_view`
- `worldsim_validate`
- `worldsim_repair_db`

`worldsim_commit_agent_turn` rejects visible text that directly includes hidden
truth strings or forbidden leak strings from the pending packet.

## Codex App Background Watcher

The VN browser does not call an LLM by itself. It only writes durable pending
jobs into the world store:

- `codex_app_server_dispatch_started`: a queued player action was dispatched to
  Codex App app-server for narrative authorship.
- `codex_app_image_generate_completed`: a visual job closed through Codex App
  `imageGeneration` and was written back into the world store.

For local Codex App play, the operator phrase `싱귤러리 월드 준비해줘` means:
start the background worker and leave it running so the browser can advance
world turns without a manual chat turn. The app-facing entrypoint is
`host-worker`:

```bash
singulari-world --store-root .world-store host-worker \
  --text-backend codex-app-server \
  --interval-ms 750
```

Codex App should remain open while the VN browser is used. The worker starts a
managed loopback `codex app-server` when no `--codex-app-server-url` is passed,
dispatches pending text turns through that websocket, and commits completed
text results back into the world store. The same worker consumes visual jobs
through Codex App `imageGeneration` and writes completion metadata. When there
is no active world or no pending work, it idles.

For a packaged app, Codex App should own this cross-platform background process
instead of relying on OS-specific schedulers such as launchd or Windows Task
Scheduler. Start it before opening the VN app, stop it when the app closes. Omit
`--world-id` in the normal app flow so it can follow whichever world the browser
creates or loads.

The primary realtime text backend is `codex-app-server`. `codex-app-poller` is a
legacy event-only contract. Use the explicit CLI backend only when the host
cannot run an app-server websocket:

```bash
singulari-world --store-root .world-store host-worker \
  --world-id <world-id> \
  --text-backend codex-exec-resume
```

The normal app loop uses `host-worker`. It prints newline-delimited JSON events
for dispatch and completion state while also closing the work itself.

```json
{"event":"codex_app_server_dispatch_started","world_id":"...","turn_id":"turn_0001","turn_status":"completed"}
{"event":"codex_app_image_generate_completed","world_id":"...","slot":"turn_cg:turn_0001","status":"completed"}
```

Image generation is not dispatched through `codex exec` or through the active
Codex chat's visual session. It is queue-based: the simulator exposes a redacted
visual job, then `host-worker` claims it, asks Codex App app-server for exactly
one `imageGeneration`, copies the returned saved PNG, writes completion
metadata, and clears the active claim.

For realtime local play inside an existing Codex thread, bind it explicitly or
pass it to `host-worker`:

```bash
SINGULARI_WORLD_CODEX_BIN=/path/to/codex \
singulari-world --store-root .world-store host-worker \
  --world-id <world-id> \
  --interval-ms 750 \
  --codex-thread-id <codex-thread-id>

singulari-world --store-root .world-store codex-thread-bind \
  --world-id <world-id> \
  --thread-id <codex-thread-id> \
  --codex-bin /path/to/codex

singulari-world --store-root .world-store codex-thread-show --world-id <world-id>
```

When `host-worker --text-backend codex-app-server` sees a pending turn, it
connects to the configured or managed Codex app-server websocket, resumes or
starts the world's dedicated Codex thread, sends the bounded realtime prompt
through `turn/start`, and waits for completion. A first successful unbound
dispatch persists the returned thread id to `codex_thread_binding.json`.

The world-specific Codex thread is the narrative working context, not the
source of truth. The worker expects Codex to compact long threads according to
normal Codex runtime behavior, and it compensates by reinjecting the bounded
world-store packet on every turn. If a bound thread cannot be resumed because it
is stale or missing, the worker clears that world's binding so the next dispatch
can start a fresh thread from the same world DB.

If no `--codex-app-server-url` is provided, `host-worker` starts
`codex app-server` on a loopback port and writes
`codex_app_server_runtime.json` under the store-root `agent_bridge` directory. Pass
an explicit URL only when the embedding host owns the app-server process.

After the background worker is ready, the browser-facing runtime is just the VN
server:

```bash
singulari-world --store-root .world-store vn-serve --port 4177
```

For Tailscale phone play, serve the same app on a Tailscale address or hostname:

```bash
singulari-world --store-root .world-store vn-serve \
  --host <tailscale-ip-or-hostname> \
  --port 4177
```

Do not use a regular LAN bind or `0.0.0.0`; the VN server allowlist is loopback
plus Tailscale.

When `host-worker --text-backend codex-exec-resume` sees a pending turn, it
starts `codex exec resume <codex-thread-id> -` with a bounded prompt and waits
for it to finish.
That worker reads the pending packet, writes an `AgentTurnResponse`, commits the
turn, and leaves only a short status line in the Codex thread. Dispatch records,
prompts, stdout, stderr, and completion status are stored under the world's
`agent_bridge/dispatches` directory so duplicate watcher ticks do not start
duplicate Codex turns.

The durable binding lives at
`worlds/<world-id>/agent_bridge/codex_thread_binding.json`. `host-worker` and
`codex-thread-bind` are the only public binding surfaces. Moving play to a new
Codex chat only requires re-running `codex-thread-bind`; the long-running worker
does not need to be restarted. Passing `--codex-thread-id` to `host-worker`
seeds or refreshes that binding for the watched world.

`codex-exec-resume` is a realtime CLI backend for hosts that do not run a
Codex app-server websocket. `codex_app_poller_turn_required` remains only a
legacy event-only contract.

No external image API is part of the runtime contract. The standalone MCP owns
job discovery and redacted prompts; Codex App owns actual image generation and
filesystem save.

This is the missing runtime layer behind the browser demo: manual Codex calls
prove the protocol, but the shipped app needs this watcher running in the
background to make player input advance without an active chat turn.
