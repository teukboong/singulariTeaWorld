# Visual Canon and Budget Policy

`singulari-world` treats image generation as stateless. Codex App owns the
actual image generation capability; the world simulator owns player-visible job
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
- Existing turn CG is reused.
- User retry creates one background retry job.
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

## Codex App Worker Contract

The simulator does not call external image APIs and does not route image
generation through `codex exec`. It exposes player-visible jobs for Codex App to
consume through its host image-generation capability.

For normal Codex App play, the `싱귤러리 월드 준비해줘` prep worker must not
consume image jobs:

```bash
singulari-world --store-root .world-store host-worker \
  --text-backend codex-app-server \
  --no-visual-jobs \
  --interval-ms 750
```

Image jobs are consumed only by Codex App's host image capability. The active
Codex chat/session-level visual generation path is not a valid worker for this
contract, and a manually generated chat image must not be recorded as a worker
success.

The primary Codex App path is MCP-driven: `worldsim_claim_visual_job` returns
structured content with `job.codex_app_call`, Codex App runs its built-in image
generation capability, and `worldsim_complete_visual_job` registers the PNG.
Manual claim/complete remains the fallback contract.

Worker loop:

1. Run `host-worker --claim-visual-jobs`, poll `agent-watch` stdout, or call
   `worldsim_visual_assets`.
2. Claim one job with `visual-job-claim` / `worldsim_claim_visual_job`.
3. Run Codex App image generation with `claim.job.prompt`,
   `claim.job.reference_paths`, and `claim.job.destination_path`.
4. Save a PNG exactly to `destination_path`.
5. Complete with `visual-job-complete` / `worldsim_complete_visual_job`.
6. On host failure or cancellation, release with `visual-job-release` /
   `worldsim_release_visual_job` instead of leaving the claim locked.

Claims live under `visual_jobs/claims/` and are created atomically. Completion
verifies PNG bytes, records metadata under `visual_jobs/completed/`, removes the
claim, and refreshes the visible manifest. If a host receives a temporary PNG
path, completion may copy it with `generated_path`.

## Hidden Truth Boundary

Visual prompts and reference lists must use player-visible records only.
Private secrets may influence internal adjudication, but they must not appear in
image prompt text, file names, asset labels, or browser-visible manifests.
