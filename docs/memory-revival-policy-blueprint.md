# Memory Revival Policy Blueprint

Status: design draft

## Problem

WebGPT-only simulation increases the importance of explicit continuity revival.
The web context window and project/session memory behavior are not as clear as
Codex App's compact loop, so the simulator must decide what to send each turn.

If the packet is too small, continuity breaks. If it is too large, prompts become
slow, expensive, repetitive, and biased by stale context.

The revival policy should be a deterministic compiler, not a grab bag and not a
new source of truth.

## Goals

1. Compile a small, relevant continuity packet for each text and image turn.
2. Make retrieval policy explicit and testable.
3. Give WebGPT text more active memory revival than Codex App used.
4. Keep image revival focused on visual references and player-visible facts.
5. Separate player-visible, inferred, private, and hidden surfaces.
6. Avoid repeatedly injecting stale or irrelevant context.
7. Make every revived item traceable to a source and reason.

## Non-Goals

- Do not dump the world DB into prompts.
- Do not rely on ChatGPT memory or project memory as source of truth.
- Do not use fallback summaries when structured state exists.
- Do not let revival create new canon.
- Do not send hidden/private context to image prompts.

## Proposed Surfaces

- policy file: `revival_policy.json`
- per-turn compiled packet: in-memory `turn_context` payload
- append-only audit: `memory_revival_events.jsonl`
- DB projection: `memory_revival_events`
- prompt sections:
  - `memory_revival.active_world_lore`
  - `memory_revival.active_relationship_graph`
  - `memory_revival.active_scene_pressure`
  - `memory_revival.active_plot_threads`
  - `memory_revival.active_character_text_design`
  - `memory_revival.active_location_graph`
  - `memory_revival.active_body_resource_state`
  - `memory_revival.active_extra_memory`
  - `image_revival.active_visual_assets`

`memory_revival_packet.json` may be useful for debug snapshots, but it should
not become canonical state. Canonical facts stay in their source files and event
logs.

## Revival Item

```json
{
  "schema_version": "singulari.revival_item.v1",
  "world_id": "stw_...",
  "turn_id": "turn_0004",
  "item_id": "revival:turn_0004:rel_gate_guard",
  "source_kind": "relationship_edge",
  "source_id": "rel:char:gate_guard->char:protagonist",
  "visibility": "player_visible",
  "reason": "current_scene_entity_and_social_permission_pressure",
  "score": 0.92,
  "payload": {
    "stance": "procedural_suspicion",
    "visible_summary": "문지기는 주인공을 절차상 의심 대상으로 본다.",
    "simulation_effect": {
      "gates": ["social_permission"]
    }
  },
  "evidence_refs": [
    {
      "source": "relationship_graph.json",
      "id": "rel:char:gate_guard->char:protagonist"
    }
  ]
}
```

## Source Kinds

Closed source kinds:

- `world_lore_entry`
- `relationship_edge`
- `scene_pressure`
- `plot_thread`
- `character_text_design`
- `location_node`
- `location_route`
- `body_resource_state`
- `remembered_extra`
- `extra_trace`
- `canon_event`
- `player_knowledge`
- `visual_asset`

## Backend Profiles

### WebGPT Text

WebGPT text should receive more active continuity than the old Codex App backend,
but still through budgets.

| Source | Budget |
| --- | ---: |
| scene pressure | 5 |
| plot threads | 5 |
| world lore | 8 |
| relationship edges | 8 |
| character text designs | 8 |
| location nodes/routes | 8 |
| body/resource state | 8 |
| remembered extras | 7 |
| recent extra traces | 5 |
| recent canon events | 8 |

### WebGPT Image

Image revival should be narrower.

| Source | Budget |
| --- | ---: |
| current render packet facts | required |
| current location visual facts | 3 |
| accepted visual assets | 7 |
| player-visible character design summaries | 3 |
| player-visible scene pressure | 2 |
| recent scene CG continuity | 1 |

No hidden/private relationship, lore, thread, or future event enters image
revival.

## Two-Stage Compilation

Revival has two stages:

1. Source revival runs before scene pressure. It selects compact durable facts
   from world state using player input, current location, recent turns, and open
   hooks.
2. Turn context assembly runs after scene pressure. It merges source revival,
   active pressures, schema budget, and backend visibility gates into the final
   prompt payload.

This avoids the circular design where memory revival both depends on pressure
and creates pressure.

## Scoring

Score each candidate with explicit reasons:

| Signal | Weight |
| --- | ---: |
| directly mentioned by player input | 1.00 |
| current location/entity match | 0.90 |
| active scene pressure source | 0.85 |
| active plot thread source | 0.80 |
| recent turn involvement | 0.65 |
| unresolved hook | 0.60 |
| relationship intensity | 0.55 |
| body/resource gate | 0.50 |
| dormant but semantically matched | 0.40 |
| stale unrelated | reject |

The exact values can be config constants, but the reason labels must be stable
for tests and audits.

## Anti-Repetition

Each revival item should record recent usage:

```json
{
  "source_id": "lore:settlement:west_gate",
  "last_revived_turn_id": "turn_0003",
  "revival_count_recent": 2,
  "last_effect": "used_in_choice_pressure"
}
```

Rules:

- repeat if the player directly touches it
- repeat if it is still an active pressure source
- compress if it was included recently but did not affect output
- suppress if stale and unrelated

Anti-repetition must not hide active constraints. It only removes stale context.

## Visibility Gates

| Target | Allowed Visibility |
| --- | --- |
| text adjudication section | player_visible, inferred_visible, private, hidden |
| text visible prose section | player_visible, inferred_visible |
| Archive View | player_visible, inferred_visible |
| image prompt | player_visible only, plus accepted reference assets |
| visual attachment | accepted reference assets only |
| docs projection | player_visible, inferred_visible, counts for hidden |

The compiler should produce separate sections instead of asking WebGPT to keep
visibility boundaries in its head.

## Revival Event

```json
{
  "schema_version": "singulari.memory_revival_event.v1",
  "world_id": "stw_...",
  "turn_id": "turn_0004",
  "event_id": "revival_event_000004",
  "backend": "webgpt_text",
  "selected_counts": {
    "world_lore_entry": 4,
    "relationship_edge": 3,
    "scene_pressure": 3,
    "remembered_extra": 2
  },
  "rejected_counts": {
    "stale_unrelated": 12,
    "visibility_blocked": 2,
    "budget_exceeded": 7
  },
  "top_reasons": [
    "current_location_match",
    "active_scene_pressure_source",
    "player_input_match"
  ],
  "created_at": "RFC3339"
}
```

This is audit metadata. It should not become player-facing lore.

## Prompt Shape

The prompt compiler should send:

1. fixed runtime contract
2. current player input
3. compact render/current state
4. active revival packet, divided by visibility and source kind
5. output schema

Do not send raw full docs, full DB rows, or previous full turns unless a selected
revival item requires a short evidence quote.

## Validation

Before sending a revival packet:

1. Validate every item source exists.
2. Validate visibility is allowed for target backend and section.
3. Validate payload shape by source kind.
4. Validate budgets.
5. Validate reason labels.
6. Validate image packets contain no hidden/private payloads.
7. Validate visual references are accepted and reference-allowed.
8. Validate no stale raw session transcript is injected.

Validation failure blocks dispatch. It should not send a fallback "summary"
packet.

## Implementation Plan

1. Add `RevivalPolicy`, `RevivalItem`, and `MemoryRevivalEvent`.
2. Add backend-specific policy profiles.
3. Add deterministic candidate collection from existing projections.
4. Add scoring and budget selection.
5. Split text packet into adjudication-only and player-visible sections.
6. Split image packet into player-visible facts and accepted visual references.
7. Add anti-repetition usage tracking.
8. Add revival audit events.
9. Add tests for visibility gates, budget selection, anti-repetition, and image
   hidden-redaction.
10. Add debug console view showing selected counts and top reasons.

## Acceptance Criteria

- WebGPT text receives enough context for continuity without full DB dumps.
- WebGPT image receives only player-visible facts and accepted references.
- Every revived item has source id, reason, visibility, and budget accounting.
- Hidden/private content is physically separated from visible prompt sections.
- Stale context is suppressed unless directly relevant.
- Dispatch fails loud if revival validation fails.
