# Agent Bridge

`singulari-world` separates durable world state from narrative authorship.

```text
player input
  -> pending turn
  -> WebGPT host-worker authors visible scene
  -> Rust validator commits
  -> VN packet renders the new scene
```

The browser only receives player-visible packets. Hidden adjudication context is
available through trusted local MCP tools and must not leak into visible prose,
Codex View, image prompts, or logs.

## MCP Tools

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

ChatGPT web uses `singulari-world-mcp-web` over Streamable HTTP. Its default
`play` profile can read player-visible state, submit player input, return an
existing CG as MCP image content, probe image-ingest shapes, and complete a
visual job from host-provided PNG base64 or HTTPS `image/png` URL. It cannot
read hidden pending-turn adjudication or directly commit agent-authored turns
unless started with `--profile trusted-local`.

## Background Worker

The VN browser does not call an LLM directly. It writes durable pending jobs
into the world store, then `vn-serve` wakes `host-worker --once`.

```bash
target/release/singulari-world --store-root .world-store host-worker \
  --text-backend webgpt \
  --visual-backend webgpt \
  --webgpt-output-mode tool-form \
  --interval-ms 750
```

The text lane calls `webgpt_research`, reuses
`agent_bridge/webgpt_conversation_binding.json`, and instructs WebGPT to use the
connected Singulari World MCP connector tools: `worldsim_next_turn_form` and
`worldsim_submit_turn_form`. Assistant text is only operator evidence; the
durable commit record created by `worldsim_submit_turn_form` is the success
source.

The image lane calls `webgpt_generate_image`, reuses separate image
conversation bindings, extracts a PNG, and completes the same visual jobs
queued by the VN packet. Text and image use separate CDP ports and profile dirs
from process start; they are not one browser window switching tools.

World creation writes locked `agent_bridge/backend_selection.json`. The valid
pair is WebGPT/WebGPT. Old local Codex App selections are legacy data and must
not revive a retired backend.

## Visual Job Contract

`worldsim_claim_visual_job` returns structured MCP content containing
`job.image_generation_call`. The image host generates from that call, writes
the PNG to `destination_path`, then calls `worldsim_complete_visual_job`.

Reference assets and scene CG are separate:

- `turn_cg:*` jobs write scene PNGs under `assets/vn/turn_cg/`.
- character, location, menu, and stage assets are source material only.
- accepted reference assets listed on a turn-CG job must be uploaded as actual
  files through `webgpt_generate_image.reference_paths`.

Turn CG must not directly expose protagonist or anchor-level characters before
their accepted character sheets exist.

## Text Design Anchors

Character text design lives in `voice_anchors`, not in visual assets. A
character anchor may define:

- `speech`: how the character forms utterances and decides what to say
- `endings`: sentence endings, clipped phrases, hesitation, and 말끝
- `tone`: distance, register, diction, and emotional temperature
- `gestures`: recurring physical tells
- `habits`: behavioral habits that should recur in prose
- `drift`: how the voice changes after visible canon events

Narrative craft is separate from character anchors. Prose style applies only to
`visible_scene.text_blocks`: sensory clue, action/reaction, deferred inference,
choice pressure, then aftertaste. The agent should not explain the style to the
player; it should surface through dialogue endings, repeated habits, paragraph
rhythm, and scene pressure.

The WebGPT text prompt carries a compact Korean style contract rather than a
second correction pass. It asks for natural Korean VN prose, rejects
translationese/report prose/overlong sentences, keeps most sentences around
25-55 Korean characters, and tells the model to split long causal chains into
sensory clue, reaction, and delayed judgment. Avoid-list wording such as
`해당`, `진행`, `확인`, `수행`, `위치하다`, `존재하다`, `~하는 것이 필요했다`,
`~하는 것으로 보였다`, and `~할 수 있었다` is prompt guidance, not a runtime
fallback filter.

## Runtime

After prep, the browser-facing runtime is just the VN server:

```bash
target/release/singulari-world --store-root .world-store vn-serve --port 4177
```

For Tailscale phone play:

```bash
target/release/singulari-world --store-root .world-store vn-serve \
  --host <tailscale-ip-or-hostname> \
  --port 4177 \
  --trusted-tailnet
```

Do not use a regular LAN bind or `0.0.0.0`. The default VN exposure mode is
loopback-only; `--trusted-tailnet` is required for Tailscale and assumes every
peer that can fetch `/app.js` is trusted enough to read the CSRF token.
