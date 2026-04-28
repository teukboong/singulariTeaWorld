# Visual Canon and Budget Policy

`singulari-world` treats image generation as host-owned. WebGPT owns the actual
image generation capability; the world simulator owns player-visible job
prompts, destination paths, reference lists, and asset manifests. Durable visual
memory lives in world records, manifests, and asset metadata.

## Goals

- Keep recurring characters, places, and world tone visually consistent.
- Avoid generating a new CG for every turn.
- Never leak hidden truth through image prompts or reference assets.
- Make generation work resumable: every job has a destination path, prompt, and
  registration rule.

## Visual Canon Layers

1. World style profile
   - Shared tone, palette, lens language, density, materiality, and negative
     prompt.
   - Created from player-visible seed and world premise only.

2. Character sheet
   - Used for recurring or major characters, not every temporary NPC.
   - Includes silhouette, hair, clothing, color anchors, expression notes, and
     negative traits.
   - Created when a character is major, repeated, or explicitly requested.
   - For protagonist and anchor-level characters, the accepted sheet is the
     permission boundary for direct scene CG exposure.

3. Location sheet
   - Used for repeated locations or first major location reveal.
   - Keeps architecture, palette, light, and spatial landmarks stable.

4. Scene CG
   - A turn-specific VN image.
   - Compiled from player-visible scene state plus selected visual canon refs.
   - Previous scene references are used only for same continuity group, not as a
     universal memory substitute.

## Budget Modes

The default public mode is `balanced`.

```yaml
visual_budget_policy:
  mode: balanced
  turn_cg_cadence_min: 5
  auto_retry_limit: 1
  max_reference_images: 3
  character_sheet_threshold: major_or_repeated
  location_sheet_threshold: repeated_location
  force_generate_on:
    - first_major_character_appearance
    - new_major_location
    - climax
    - combat_start
    - revelation
    - user_request
```

## Turn CG Decision

The VN packet should choose one of these actions:

- `off`: no image work for this turn.
- `reuse_last`: keep the current or previous scene image.
- `generate_scene`: create one turn CG.
- `generate_sheet_then_scene`: create missing major sheets first, then scene.
- `background_only`: create a location/background layer without character
  consistency refs.

Default balanced behavior:

- Initial turn uses world menu/stage backgrounds.
- Veiled characters do not get reference sheet jobs until the story makes them
  player-visible.
- Protagonist and anchor-level characters do not appear directly in scene CG
  until their accepted character sheet exists. Before that, use POV,
  environment-only, off-screen, shadow, or cropped non-identifying framing.
- Existing turn CG is reused until the user explicitly requests regeneration.
- User retry creates one background retry job even when an image already exists;
  completion overwrites the turn PNG and clears the retry marker.
- Codex/log/settings turns do not generate CG.
- Major transitions and explicit visual modes can generate immediately.
- Normal story turns use sparse cadence: roughly one CG per
  `turn_cg_cadence_min` turns.

## Reference Priority

When the implementation supports reference image attachment, choose references
in this order and never exceed `max_reference_images`:

1. Accepted character sheets for visible major characters.
2. Accepted location sheet for the current location.
3. Previous scene CG from the same continuity group.

Prompt-only generation remains acceptable for menu backgrounds, base stage
backgrounds, abstract interludes, and one-off scenery.

## Major Character Design Gate

Direct depiction of major characters is gated by accepted design assets, not by
the text scene alone. If a turn includes an unresolved protagonist or
anchor-level character, the scene CG prompt must avoid identifiable direct
exposure: faces, full-body front views, distinctive outfits, and clear
silhouettes are blocked until the accepted character sheet exists.

Allowed pre-sheet framing:

- player POV with the character off-camera
- environment or prop-focused composition
- shadow or reflection that cannot identify the final design
- cropped hands, back-of-head, or non-identifying fragments

Character sheet jobs are the correct way to reveal and stabilize major designs.
Once the sheet is saved under `assets/vn/character_sheets/`, scene CG jobs may
attach it as the first-priority reference and depict the character directly.
The sheet itself remains source material. It must not be copied into
`assets/vn/turn_cg/` or displayed as the turn scene image. WebGPT preserves that
boundary by using a separate reference-asset conversation for character/location
design jobs and reserving the turn-CG conversation for scene images only. When
scene CG generation needs accepted reference assets, those files must be
attached to the WebGPT image request as images, not merely mentioned by local
path in the prompt.

## WebGPT Worker Contract

The simulator does not call external image APIs and does not route image
generation through `codex exec`. It exposes player-visible jobs for WebGPT to
consume through the host-worker image lane.

For normal play, the `싱귤러리 월드 준비해줘` worker consumes both text turns and
visual jobs through WebGPT:

```bash
target/release/singulari-world --store-root .world-store host-worker \
  --text-backend webgpt \
  --visual-backend webgpt \
  --interval-ms 750
```

Image jobs are consumed only by the WebGPT image lane. The worker claims one
job, requests exactly one `webgpt_generate_image`, copies the extracted PNG, and
completes the job. The active Codex chat/session-level visual generation path
is not a valid worker for this contract.

The MCP path is host-neutral: `worldsim_claim_visual_job` returns structured
content with `job.image_generation_call`, the image host generates the PNG, and
`worldsim_complete_visual_job` registers it.

ChatGPT web MCP uses a narrower image contract. `worldsim_current_cg_image`
returns an already saved PNG as MCP image content so the host/model can inspect
the current CG. `worldsim_probe_image_ingest` records whether the host can pass
generated images back as `image_base64`, `image_url`, `resource_uri`, or
`file_id`; it records only shape/byte-count metadata and does not persist image
payloads or complete jobs. `worldsim_complete_visual_job_from_base64` is the
first promoted ingest path: it accepts only PNG base64 or
`data:image/png;base64,...`, stages it temporarily, and then reuses the normal
visual-job completion verifier. `worldsim_complete_visual_job_from_url` applies
the same verifier after fetching an HTTPS `image/png` URL with redirect, host,
and byte-count limits.

Worker loop:

1. `host-worker` or `worldsim_claim_visual_job` claims one job.
2. WebGPT or the host image capability runs generation from
   `claim.job.image_generation_call`.
3. The returned saved PNG is copied to `destination_path`.
4. Completion metadata is written through `complete_visual_job` /
   `worldsim_complete_visual_job`.

Claims live under `visual_jobs/claims/` and are created atomically. Completion
verifies PNG bytes, records metadata under `visual_jobs/completed/`, removes the
claim, and refreshes the visible manifest. If a host receives a temporary PNG
path, completion may copy it with `generated_path`.

## Hidden Truth Boundary

Visual prompts and reference lists must use player-visible records only.
Private secrets may influence internal adjudication, but they must not appear in
image prompt text, file names, asset labels, or browser-visible manifests.
