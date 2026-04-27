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

- `agent_turn_pending`: a player choice/freeform action is waiting for narrative
  authorship.
- `visual_job_pending`: a menu/stage/turn/character/location image job is waiting
  for Codex App's built-in image generation capability.

For local Codex App play, the operator phrase `싱귤러리 월드 준비해줘` means:
start the background worker and leave it running so the browser can advance
world turns without a manual chat turn. The app-facing entrypoint is
`host-worker`:

```bash
singulari-world --store-root .world-store host-worker \
  --text-backend codex-app-server \
  --no-visual-jobs \
  --interval-ms 750
```

Codex App should remain open while the VN browser is used. The worker starts a
managed loopback `codex app-server` when no `--codex-app-server-url` is passed,
dispatches pending text turns through that websocket, and commits completed
text results back into the world store. Prep mode passes `--no-visual-jobs`
because image work must be consumed by Codex App's host image capability, not by
the active Codex chat session. When there is no active world or no pending text
work, it idles.

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

The lower-level watcher remains available for hosts that only need raw events:

```bash
singulari-world --store-root .world-store agent-watch --world-id <world-id>
```

For a one-shot poll that the app can schedule itself:

```bash
singulari-world --store-root .world-store agent-watch --world-id <world-id> --once
```

The command prints newline-delimited JSON events. Codex App can subscribe to
stdout and route each event to the right internal worker. Visual jobs carry a
`codex_app_call` object; Codex App's host image worker should run that host
capability, save the PNG exactly to `destination_path`, then refresh
`worldsim_current` or `worldsim_visual_assets`.

```json
{"event":"agent_turn_pending","world_id":"...","turn_id":"turn_0001","pending_ref":"..."}
{"event":"visual_job_pending","world_id":"...","slot":"stage_background","tool":"codex_app.image.generate","codex_app_call":{"capability":"image_generation","destination_path":"..."}}
```

Image generation is not dispatched through `codex exec` or through the active
Codex chat's visual session. It is queue-based: the simulator exposes a redacted
visual job, then Codex App's host layer consumes the job with its image
generation capability and saves the PNG. Until that packaged host worker exists,
use the manual claim/complete contract below.

The manual contract remains:

```bash
singulari-world --store-root .world-store visual-job-claim \
  --world-id <world-id> \
  --slot <slot> \
  --json

# Codex App consumes the job with codex_app.image.generate and saves a PNG to
# claim.job.destination_path.

singulari-world --store-root .world-store visual-job-complete \
  --world-id <world-id> \
  --slot <slot> \
  --claim-id <claim-id> \
  --json
```

`visual-job-claim` writes an atomic claim under
`worlds/<world-id>/visual_jobs/claims/`, so multiple workers do not generate the
same asset. `visual-job-complete` verifies that the result is a real PNG, writes
completion metadata under `visual_jobs/completed/`, removes the active claim,
and refreshes the manifest so the VN server can pick up the asset on the next
packet refresh. If Codex App receives a generated PNG at a temporary path, pass
`--generated-path <png>` to copy it into the job destination during completion.
If the host generation fails or is cancelled, use `visual-job-release --slot
<slot>` / `worldsim_release_visual_job` so the job can be claimed again.

For realtime local play inside an existing Codex thread, pass that thread id to
the watcher once, or bind it explicitly:

```bash
SINGULARI_WORLD_CODEX_BIN=/path/to/codex \
singulari-world --store-root .world-store agent-watch \
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
`agent-watch` read that file on every tick, so moving play to a new Codex chat
only requires re-running `codex-thread-bind`; the long-running watcher does not
need to be restarted. Passing `--codex-thread-id` to `host-worker` or
`agent-watch` seeds or refreshes that binding for the watched world.

`codex-exec-resume` is a realtime CLI backend for hosts that do not run a
Codex app-server websocket. `codex_app_poller_turn_required` remains only a
legacy event-only contract.

No external image API is part of the runtime contract. The standalone MCP owns
job discovery and redacted prompts; Codex App owns actual image generation and
filesystem save.

This is the missing runtime layer behind the browser demo: manual Codex calls
prove the protocol, but the shipped app needs this watcher running in the
background to make player input advance without an active chat turn.
