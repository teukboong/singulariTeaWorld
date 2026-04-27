# Host Worker Contract

`singulari-world` keeps the simulator standalone. It owns world storage,
pending-turn records, visual job records, and redacted prompts. The embedding
host owns Codex App process lifetime, model session dispatch, and image
generation.

The host worker is the single background process between those systems.

## Codex App Operator Prep

When a Codex App operator says `싱귤러리 월드 준비해줘`, start the worker below
and keep Codex App open while the VN browser is used:

```bash
singulari-world --store-root .world-store host-worker \
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

## Runtime Path

`host-worker` has one text dispatch path:

- Connect to an explicit `--codex-app-server-url`, or start a managed loopback
  `codex app-server`.
- Read the active world from `.world-store/active_world.json`.
- When `pending_turn.json` exists, resume the saved world thread or start a new
  Codex thread.
- Send the bounded world prompt through `turn/start`.
- Persist the returned `thread_id` in
  `worlds/<world-id>/agent_bridge/codex_thread_binding.json`.
- Commit the agent-authored visible turn through the normal world-store
  contract.

That thread is the world's warm narrative context. The world DB and JSON/JSONL
files remain source of truth and are injected into every turn, so Codex
compaction or thread rebuilds do not erase canon.

If `thread/resume` fails for a stale or missing thread, the worker clears only
that world's binding. The next tick starts a fresh thread from the same world
store.

Image generation is the same worker loop:

- Claim one pending visual job.
- Ask Codex App app-server for exactly one `imageGeneration`.
- Read the returned `savedPath`.
- Copy the PNG to `destination_path`.
- Complete the visual job and clear the claim.

The simulator binary never calls external image providers, `~/.codex` skills,
the active chat visual session, shell drawing scripts, SVG placeholders, or
local provider keys.

## Process Supervision

Use the release binary for long-running play sessions:

```bash
cargo build --locked --release --bin singulari-world --bin singulari-world-mcp

target/release/singulari-world --store-root .world-store host-worker \
  --interval-ms 750

target/release/singulari-world --store-root .world-store vn-serve --port 4177
```

If a macOS host supervises the worker with `launchctl`, do not rely on the
interactive shell PATH. Pass `--codex-bin /absolute/path/to/codex` and set PATH
so `/usr/bin/env node` can resolve `node`; the npm Codex launcher depends on
that lookup. A suitable launchd environment includes the directories containing
`node`, `codex`, and the system tools:

```text
PATH=/usr/local/bin:/Users/<user>/.npm/bin:/usr/bin:/bin:/usr/sbin:/sbin
```

When replacing an old runtime, stop the old `vn-serve`, `host-worker`, and any
managed `codex app-server --listen ws://127.0.0.1:<port>` child before starting
the new worker. Stale `dispatching` records are durable by design; if a worker
dies before writing a terminal dispatch record, remove only that failed
dispatch record before retrying the same pending turn.

## Reference CLI

Start a one-shot worker for the active or explicit world:

```bash
singulari-world host-worker --world-id <world-id> --once
```

Run the normal long-lived app worker:

```bash
singulari-world --store-root .world-store host-worker --interval-ms 750
```

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
{"event":"worker_started","world_id":"stw_example","text_backend":"codex-app-server"}
```

Waiting for a world:

```json
{"event":"worker_waiting_for_active_world","text_backend":"codex-app-server"}
```

Realtime text dispatch:

```json
{"event":"codex_app_server_dispatch_started","world_id":"stw_example","turn_id":"turn_0001","thread_id":"019d...","turn_status":"completed"}
```

Image job completed:

```json
{"event":"codex_app_image_generate_completed","world_id":"stw_example","slot":"turn_cg:turn_0001","status":"completed","saved_path":"/path/to/generated.png"}
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
- Missing or unstartable Codex app-server is fatal for worker dispatch. There is
  no secondary text backend.
- Visual jobs close only through Codex App `imageGeneration` and the returned
  saved file.
- Dispatch records and visual claims are durable world-store files, so worker
  restarts do not duplicate already-started jobs.
