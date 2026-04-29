# Visual Asset Graph Blueprint

Status: design draft

## Problem

The simulator now has multiple visual artifact types:

- menu background
- stage background
- character design sheets
- location design sheets
- turn CG
- generated images fetched from WebGPT sessions
- reference assets attached to WebGPT image prompts

The current job ledger can track generation, but the simulator still needs a
clear graph that says which visual asset is displayable, which is reference-only,
which entity/location/lore it belongs to, and which assets may influence a turn
CG.

Without that graph, reference images can be mistaken for scene CG, unfinished
character designs can leak into player-facing display, and image continuity
depends too much on prompt prose.

## Goals

1. Model visual assets as first-class per-world graph nodes.
2. Separate reference assets from display assets.
3. Track entity/location/lore/relationship refs for visual continuity.
4. Enforce major-character design gates before direct CG depiction.
5. Let WebGPT image backend reuse image session continuity safely.
6. Prevent fetched attached images from being treated as generated output.
7. Keep image prompts player-visible and hidden-truth clean.

## Non-Goals

- Do not make visual assets source of truth for world lore.
- Do not infer canon from image pixels.
- Do not display character sheets as scene CG.
- Do not attach every available asset to every image prompt.
- Do not use visual continuity to reveal hidden identities or future events.

## Proposed Surfaces

- file source: `visual_asset_graph.json`
- append-only event source: `visual_asset_events.jsonl`
- existing manifest bridge: `visual_assets.json`
- DB projection: `visual_assets`, `visual_asset_events`, `visual_asset_refs`
- revival projection for image backend: `image_revival.active_visual_assets`
- prompt section: `visual_asset_contract`
- UI projection: console "Visual Assets" and CG display

## Visual Asset Node

```json
{
  "schema_version": "singulari.visual_asset_node.v1",
  "world_id": "stw_...",
  "asset_id": "asset:character_sheet:char:protagonist",
  "slot": "character_sheet:char:protagonist",
  "artifact_kind": "character_design_sheet",
  "canonical_use": "reference_only",
  "display_allowed": false,
  "reference_allowed": true,
  "generation_status": "completed",
  "path": "assets/vn/character_sheets/char_protagonist.png",
  "entity_refs": ["char:protagonist"],
  "location_refs": [],
  "lore_refs": [],
  "relationship_refs": [],
  "source_job_id": "reference_asset:character_sheet:char:protagonist",
  "continuity_notes": [
    "use as visual reference only after design acceptance"
  ],
  "visibility": "private_reference",
  "created_turn_id": "turn_0000",
  "last_changed_turn_id": "turn_0000"
}
```

## Artifact Kinds

Use the existing Rust `VisualArtifactKind` as the source enum.

| Kind | Canonical Use |
| --- | --- |
| `ui_background` | display or reference, depending on slot |
| `character_design_sheet` | reference only |
| `location_design_sheet` | reference only |
| `scene_cg` | display |

V1 should keep this enum small. New image classes need a schema change, not
free-form strings.

## Canonical Use

| Use | Meaning |
| --- | --- |
| `display` | may appear in VN scene or menu |
| `reference_only` | may be attached to image generation but not displayed as CG |
| `source_material` | retained for continuity but not auto-attached |
| `retired` | should not be used |

`display_allowed` and `reference_allowed` must agree with `canonical_use`.
Contradictions fail validation.

## Character Design Gate

Major or recurring characters must not be directly depicted in scene CG until
their design state is accepted.

Design states:

- `not_started`
- `draft_reference`
- `needs_review`
- `accepted_reference`
- `retired`

Rules:

- `not_started`: do not directly depict the character in CG.
- `draft_reference`: may exist as private/reference material, not direct CG.
- `needs_review`: do not use in turn CG until accepted.
- `accepted_reference`: may attach as reference and allow direct depiction.
- `retired`: do not use.

The image prompt may still depict indirect presence: silhouette, hand, shadow,
off-screen voice, object trace, or environmental effect when player-visible.

## Generated Output Detection

WebGPT image sessions may contain:

- generated output images
- user-uploaded or runtime-attached reference images
- UI thumbnails
- stale images from previous prompts

The ingest path must accept only generated outputs for the active visual job.

Required evidence:

- active job id
- prompt correlation id or generation turn marker
- image appears after the prompt send time
- image belongs to assistant/generated output container
- image is not one of the attached reference asset URLs/files

If these checks fail, ingestion fails loud. It must not complete the visual job
with an attached reference image.

## Asset Event

```json
{
  "schema_version": "singulari.visual_asset_event.v1",
  "world_id": "stw_...",
  "event_id": "visual_asset_event_000004",
  "turn_id": "turn_0001",
  "asset_id": "asset:scene_cg:turn_0001",
  "change": "completed",
  "artifact_kind": "scene_cg",
  "canonical_use": "display",
  "source_job_id": "scene_cg:turn_cg:turn_0001",
  "entity_refs": [],
  "location_refs": ["place:west_gate"],
  "lore_refs": ["lore:settlement:west_gate"],
  "relationship_refs": [],
  "visibility": "player_visible",
  "evidence_refs": [
    {
      "source": "visual_job_completion",
      "job_id": "scene_cg:turn_cg:turn_0001"
    }
  ],
  "created_at": "RFC3339"
}
```

Allowed changes:

- `job_created`
- `claimed`
- `completed`
- `released`
- `accepted_reference`
- `rejected_reference`
- `display_enabled`
- `display_disabled`
- `retired`

## Attachment Selection

For each image job, attach at most:

| Asset Type | Budget |
| --- | ---: |
| accepted character references | 3 |
| location references | 2 |
| UI/style references | 1 |
| previous scene CG | 1 |

Ranking:

1. direct entity/location refs in current scene
2. accepted design state
3. same location or route
4. same visual style profile
5. recent turn CG continuity

Do not attach unaccepted major character sheets.

## Prompt Contract

Image prompts may use:

- player-visible render packet data
- active location graph visible fields
- accepted visual references
- world visual style profile
- player-visible lore and relationship summaries

Image prompts must not use:

- hidden state
- private relationship summaries
- unresolved major-character designs
- future events
- secret faction symbols
- text style examples as visual facts

## Validation

Before completing a visual job:

1. Validate job id and slot match active pending job.
2. Validate artifact kind matches expected session kind.
3. Validate generated output evidence.
4. Validate attached references are not selected as output.
5. Validate canonical use flags.
6. Validate major-character design gate.
7. Validate visibility and hidden-truth redaction.
8. Validate file path stays inside world asset root.

No fallback completion with "best available image."

## Implementation Plan

1. Add visual asset graph/event structs.
2. Materialize graph from existing visual manifest and job ledger.
3. Add generated-output evidence fields to WebGPT image dispatch records.
4. Add reference attachment selection with design-gate checks.
5. Add visual asset graph validation.
6. Update current CG selection to keep last completed scene CG until next one
   completes.
7. Render visual asset status in console UI.
8. Add repair path from visual asset events and existing files.
9. Add tests for reference-vs-output rejection, major-character gate, canonical
   use validation, and last-CG persistence.

## Acceptance Criteria

- Character sheets and location sheets are never displayed as scene CG.
- WebGPT image ingest accepts only generated output for the active job.
- Scene CG can attach accepted references without leaking hidden state.
- Major characters are not directly depicted before accepted design.
- Current CG remains visible until a newer scene CG completes.
- Visual graph can explain why an asset was displayed, attached, or rejected.
