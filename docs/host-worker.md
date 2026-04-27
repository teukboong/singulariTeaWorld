# Host Worker Contract

`singulari-world` keeps the simulator standalone. It owns world storage,
pending-turn records, visual job records, and redacted prompts. The embedding
host owns model session dispatch and image generation.

The host worker is the process boundary between those two systems.

## Codex App Operator Prep

When a Codex App operator says `싱귤러리 월드 준비해줘`, start the worker below
and keep Codex App open while the VN browser is used:

```bash
singulari-world --store-root .world-store host-worker \
  --text-backend codex-app-server \
  --interval-ms 750
```

This prepares the managed loopback `codex app-server`, realtime text dispatch,
and image queue consumption through Codex App `imageGeneration`. It is safe to
start before a world exists; the worker emits `worker_waiting_for_active_world`
until the browser creates or loads one. Idle ticks do not start model turns.

After that, the web-facing process can be started independently:

```bash
singulari-world --store-root .world-store vn-serve --port 4177
```

The web app writes pending jobs into the world store. The already-running worker
consumes those jobs and writes visible results back, so play can continue from
the browser as long as Codex App and `host-worker` remain alive.

## Dispatch Backends

Text turn dispatch has these backend modes:

- `codex-app-poller`
  - Legacy contract path.
  - The embedding app runs a Codex App chat-session poller. That poller reads
    the emitted pending-turn event, lets the active chat agent author the
    `AgentTurnResponse`, then commits it through the CLI or MCP.
  - The reference CLI does not reverse-engineer private app endpoints and does
    not claim that a direct local session API exists. This backend emits
    `codex_app_poller_turn_required` and stops there.
  - `host-session-api` remains a deprecated CLI alias for older local scripts.

- `codex-app-server`
  - Primary realtime path for packaged apps.
  - Uses the official Codex `app-server` websocket protocol:
    `thread/start` or `thread/resume`, then `turn/start`.
  - The host worker keeps idle cost at zero model tokens. It only starts a
    Codex turn when `pending_turn.json` exists.
  - If a world has no `codex_thread_binding.json`, the worker starts a
    dedicated world agent thread and stores the returned thread id. Later turns
    resume that thread.
  - That thread remains the world's narrative working context. Codex may compact
    it like any other thread, so the worker still injects world-store state on
    every turn; the world DB is source of truth, and the thread is a warm
    context cache.
  - If `thread/resume` fails for a stale or missing thread, the worker clears
    only that world's binding. The next tick can rebuild a fresh thread from the
    world store instead of staying stuck on the dead context.
  - If no explicit URL is provided, the worker starts `codex app-server` on a
    loopback port, records `codex_app_server_runtime.json` in the store-root
    `agent_bridge` directory, and kills that child process when the worker exits.
  - An embedding host may still pass an explicit websocket URL when it owns the
    app-server process itself.

- `codex-exec-resume`
  - Reference on-demand CLI path.
  - Uses `codex exec resume <thread-id> -` after a world is bound with
    `codex-thread-bind` or `--codex-thread-id`.
  - Dispatch records prevent duplicate turns for the same world/turn/thread.

- `manual`
  - Development path.
  - Emits the exact pending turn and command hints without dispatching.

Image generation has two production entrypoints:

- `singulari-world-mcp`
  - Primary Codex App path.
  - `worldsim_claim_visual_job` returns structured MCP content with
    `job.codex_app_call`.
  - Codex App consumes that structured call with its built-in image generation
    capability, saves the PNG to `destination_path`, then calls
    `worldsim_complete_visual_job`.
  - This is the standalone repo contract; it does not depend on `~/.codex`
    skills or external provider keys.

- `codex-app-server`
  - Standalone Codex App app-server path.
  - Starts or connects to Codex App `app-server`, requests exactly one
    `imageGeneration` item, reads its `savedPath`, then completes the visual job.
  - Does not use external provider keys, `~/.codex` skills, shell drawing
    fallbacks, or the active Codex chat visual session.

There is no `codex exec` image fallback.

## Reference CLI

Start a one-shot host worker:

```bash
singulari-world host-worker \
  --world-id <world-id> \
  --once \
  --text-backend codex-app-server
```

Run the explicit `codex exec resume` text backend:

```bash
singulari-world codex-thread-bind \
  --world-id <world-id> \
  --thread-id <codex-thread-id> \
  --codex-bin "$(command -v codex)"

singulari-world host-worker \
  --world-id <world-id> \
  --text-backend codex-exec-resume \
  --interval-ms 750
```

Run the websocket app-server backend:

```bash
singulari-world host-worker \
  --text-backend codex-app-server \
  --interval-ms 750
```

`codex-app-server` may run without `codex-thread-bind`; the first successful
dispatch stores a world-specific thread binding automatically. Passing
`--codex-thread-id` makes it resume an existing thread instead. The websocket
URL is runtime plumbing, while the durable narrative context is the saved
`thread_id`.

Omit `--world-id` in an embedding app. The worker will emit
`worker_waiting_for_active_world` until the browser creates or loads a world,
then it follows the active world binding.

Pass `--codex-app-server-url ws://127.0.0.1:<port>` only when the embedding host
already owns the app-server process.

Run with host-owned image jobs:

```bash
singulari-world host-worker \
  --world-id <world-id> \
  --text-backend codex-app-server \
  --interval-ms 750
```

The worker claims at most one unclaimed visual job per tick, asks Codex App
`app-server` for one `imageGeneration`, copies the returned saved file into the
world store, and writes completion metadata.

## JSONL Events

Every host worker event is one JSON object per line with schema version
`singulari.host_worker_event.v1`.

Startup:

```json
{"event":"worker_started","world_id":"stw_example","text_backend":"codex-exec-resume"}
```

Legacy text path waiting for the embedding app:

```json
{"event":"codex_app_poller_turn_required","world_id":"stw_example","turn_id":"turn_0001","consumer_contract":"codex_app_thread_poller","official_host_api":false}
```

Realtime websocket text dispatch:

```json
{"event":"codex_app_server_dispatch_started","world_id":"stw_example","turn_id":"turn_0001","thread_id":"019d...","turn_status":"completed"}
```

Fallback text dispatch:

```json
{"event":"codex_exec_dispatch_started","world_id":"stw_example","turn_id":"turn_0001","pid":12345}
```

Manual text path:

```json
{"event":"manual_agent_turn_required","world_id":"stw_example","turn_id":"turn_0001"}
```

App-server image job completed:

```json
{"event":"codex_app_image_generate_completed","world_id":"stw_example","slot":"turn_cg:turn_0001","status":"completed","saved_path":"/path/to/generated.png"}
```

App-server image job failed:

```json
{"event":"codex_app_image_generate_failed","world_id":"stw_example","slot":"turn_cg:turn_0001","status":"failed"}
```

Idle:

```json
{"event":"worker_idle","world_id":"stw_example"}
```

## Failure Rules

- Hidden adjudication context never appears in host worker image prompts or VN
  packets.
- `codex-app-poller` fails closed in the reference CLI. It only emits a poller
  action event; the Codex App chat session must consume it.
- `codex-app-server` starts a managed loopback app-server when no explicit URL
  is provided. Spawn/listen failures are fatal because silently falling back to
  private desktop IPC would make dispatch state ambiguous.
- `codex-exec-resume` requires a bound thread. Without one, it emits
  `codex_exec_dispatch_blocked`.
- Visual jobs are never generated by the simulator binary, the active Codex chat
  visual session, `~/.codex` skills, external provider keys, shell drawing
  fallbacks, or placeholders. The `codex-app-server` visual backend delegates to
  Codex App `imageGeneration` and completes the claimed job from the returned
  saved file.
- Dispatch records and visual claims are durable world-store files, so worker
  restarts do not duplicate already-started jobs.
