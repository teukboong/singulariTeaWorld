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

For a packaged app, Codex App should own the cross-platform background process
instead of relying on OS-specific schedulers such as launchd or Windows Task
Scheduler. The app-facing entrypoint is `host-worker`. Start it when the VN app
opens, stop it when the app closes:

```bash
singulari-world --store-root .world-store host-worker --world-id <world-id>
```

The intended main text backend is `host-session-api`; the reference CLI does not
call private app endpoints, so it emits `host_session_turn_required` for the
embedding host. Use the explicit fallback backend when the host wants the CLI to
dispatch through the public Codex command:

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
`codex_app_call` object; Codex App should run that host capability, save the PNG
exactly to `destination_path`, then refresh `worldsim_current` or
`worldsim_visual_assets`.

```json
{"event":"agent_turn_pending","world_id":"...","turn_id":"turn_0001","pending_ref":"..."}
{"event":"visual_job_pending","world_id":"...","slot":"stage_background","tool":"codex_app.image.generate","codex_app_call":{"capability":"image_generation","destination_path":"..."}}
```

Image generation is not dispatched through `codex exec`. It is a Codex App host
capability. The stable worker contract is:

```bash
singulari-world --store-root .world-store visual-job-claim \
  --world-id <world-id> \
  --slot <slot> \
  --json

# Codex App host runs codex_app.image.generate with the returned prompt and
# saves a PNG to claim.job.destination_path.

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

When `host-worker --text-backend codex-exec-resume` sees a pending turn, it
starts a detached `codex exec resume <codex-thread-id> -` worker with a bounded
prompt.
That worker reads the pending packet, writes an `AgentTurnResponse`, commits the
turn, and leaves only a short status line in the Codex thread. Dispatch records,
prompts, stdout, and stderr are stored under the world's `agent_bridge/dispatches`
directory so duplicate watcher ticks do not start duplicate Codex turns.

The durable binding lives at
`worlds/<world-id>/agent_bridge/codex_thread_binding.json`. `agent-watch` reads
that file on every tick, so moving play to a new Codex chat only requires
re-running `codex-thread-bind`; the long-running watcher does not need to be
restarted. Passing `--codex-thread-id` to `agent-watch` seeds or refreshes that
binding for the watched world.

This is a realtime fallback for hosts that do not expose an official session
dispatch API. A packaged first-party host can replace the `codex exec resume`
dispatch with an internal session event while preserving the same pending-turn
and dispatch-record contract.

No external image API is part of the runtime contract. The standalone MCP owns
job discovery and redacted prompts; Codex App owns actual image generation and
filesystem save.

This is the missing runtime layer behind the browser demo: manual Codex calls
prove the protocol, but the shipped app needs this watcher running in the
background to make player input advance without an active chat turn.
