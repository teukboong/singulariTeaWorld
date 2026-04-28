# Host Worker Contract

`host-worker` is the execution tick between the VN browser and WebGPT. The
browser writes durable pending text and visual jobs into the world store;
`host-worker` consumes those jobs, validates the result, and writes committed
state back. There is no Codex App backend and no managed `codex app-server`
runtime in this package.

## Normal Prep

When the operator says `싱귤러리 월드 준비해줘`, build the release binary and
start the VN server:

```bash
cargo build --locked --release --bin singulari-world --bin singulari-world-mcp
target/release/singulari-world --store-root .world-store vn-serve --port 4177
```

`vn-serve` wakes `host-worker --once` whenever the active world has pending
work. The one-shot worker exits after the bounded tick. A long-running worker is
only for diagnostics or custom embedding hosts; normal play must not depend on
`launchd`/KeepAlive.

## WebGPT Text Lane

The text lane:

- finds `webgpt-mcp-checkout/scripts/webgpt-mcp.sh`, or uses
  `--webgpt-mcp-wrapper` / `SINGULARI_WORLD_WEBGPT_MCP_WRAPPER`;
- reuses one world-scoped ChatGPT URL in
  `agent_bridge/webgpt_conversation_binding.json`;
- builds a redacted pending-turn prompt with active memory revival from
  `resume_pack`, Archive View, recall hits, and recent world.db updates;
- calls `webgpt_research`;
- extracts exactly one `AgentTurnResponse` JSON from `answer_markdown`;
- commits through the Rust validator and hidden-truth redaction checks.

`--webgpt-turn-command` can replace the built-in MCP adapter, but the output
contract is still one validated `AgentTurnResponse`.

## WebGPT Image Lane

The image lane:

- claims one pending visual job;
- calls WebGPT MCP `webgpt_generate_image`;
- uploads accepted reference asset files through `reference_paths`;
- receives an extracted generated PNG path;
- copies/saves the PNG to `destination_path`;
- completes the visual job and clears the claim.

Turn CG and reference assets use separate world-scoped ChatGPT URLs:

- `agent_bridge/webgpt_image_conversation_binding.json` for `turn_cg:*`
- `agent_bridge/webgpt_reference_asset_conversation_binding.json` for character,
  location, menu, and stage design assets

Reference assets are source material only. They must not be displayed as scene
CG, and they must not share the turn-CG conversation.

Turn CG has a major-character design gate. If a protagonist or anchor-level
character does not yet have an accepted character sheet under
`assets/vn/character_sheets/`, scene prompts must use POV/environment/off-screen
framing instead of direct faces, full-body front views, distinctive outfits, or
identifiable silhouettes.

## Parallelism

WebGPT text and image lanes are separate browser sessions from process start:

- text CDP port: `9238`
- image CDP port: `9239`
- separate profile roots under `~/.hesperides/singulari-world/webgpt/`

Startup fails if both lanes share a port or profile. The worker dispatches
already-pending text and visual work in parallel. If text commit creates a new
turn-CG job during the same tick, the worker claims one new visual job before
exiting.

## Backend Selection

Worlds created by the VN launcher write a locked
`agent_bridge/backend_selection.json`. The only valid backend pair is
WebGPT/WebGPT. Old `codex-app-server` selections are legacy local data and must
not start a retired runtime.

`--visual-backend none` is allowed only for explicit text-only validation. It
must leave pending CG jobs queued and untouched.

## Reference CLI

One-shot worker:

```bash
singulari-world --store-root .world-store host-worker --world-id <world-id> --once
```

Diagnostic loop:

```bash
singulari-world --store-root .world-store host-worker \
  --text-backend webgpt \
  --visual-backend webgpt \
  --interval-ms 750
```

## JSONL Events

Every host worker event is one JSON object per line with schema version
`singulari.host_worker_event.v1`.

```json
{"event":"worker_started","world_id":"stw_example","text_backend":"webgpt"}
{"event":"webgpt_dispatch_started","world_id":"stw_example","turn_id":"turn_0001","turn_status":"completed"}
{"event":"webgpt_image_generate_completed","world_id":"stw_example","slot":"turn_cg:turn_0001","status":"completed","generated_path":"/path/to/generated.png"}
{"event":"worker_idle","world_id":"stw_example","text_backend":"webgpt"}
```

## Failure Rules

- Hidden adjudication context never appears in image prompts, VN packets, or
  player-visible logs.
- Missing WebGPT MCP wrapper or unusable WebGPT session is fatal for WebGPT text
  or visual dispatch.
- The worker does not silently fall back to deterministic script output,
  placeholder images, Codex App runtime, or the active chat visual session.
- Visual jobs close only through WebGPT `webgpt_generate_image` and the normal
  visual-job completion verifier.
- Dispatch records and visual claims are durable world-store files, so worker
  restarts do not duplicate already-started jobs.
