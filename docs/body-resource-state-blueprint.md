# Body and Resource State Blueprint

Status: design draft

## Problem

The simulator has protagonist state and visible scene text, but body and
resources are not yet a strong simulation surface. Without structured physical
and material state, scenes can ignore fatigue, injury, cold, hunger, money,
tools, documents, and visible condition.

That makes choices feel weightless.

## Goals

1. Make body and resources first-class constraints.
2. Keep the model narrative-facing, not RPG stat clutter.
3. Let physical state alter scene pressure, choices, dialogue reactions, and
   prose rhythm.
4. Track visible condition separately from hidden/internal condition.
5. Preserve evidence and turn history for state changes.
6. Avoid silent fallback defaults when a state change is malformed.

## Non-Goals

- Do not create a full combat system.
- Do not expose numeric internals by default.
- Do not make body/resource state dominate every scene.
- Do not use resources as arbitrary punishment.
- Do not invent inventory items from prose unless the response proposes them
  through structured fields and evidence.

## Proposed Surfaces

- file source: `body_resource_state.json`
- append-only event source: `body_resource_events.jsonl`
- DB projection: `body_resource_events`, `active_body_resource_state`
- revival projection: `memory_revival.active_body_resource_state`
- player projection: Archive View "Condition and Possessions"
- prompt section: `body_resource_contract`

## State Model

```json
{
  "schema_version": "singulari.body_resource_state.v1",
  "world_id": "stw_...",
  "entity_id": "char:protagonist",
  "body": {
    "fatigue": 2,
    "hunger": 1,
    "cold": 2,
    "injury": [
      {
        "injury_id": "injury:left_wrist_bruise",
        "visibility": "player_visible",
        "severity": 1,
        "summary": "left wrist aches after waking on wet ground",
        "mechanical_effect": ["fine_grip_uncomfortable"],
        "created_turn_id": "turn_0000"
      }
    ],
    "visible_condition": [
      "mud on clothing",
      "left wrist guarded"
    ]
  },
  "resources": {
    "money": {
      "state": "unknown_to_player",
      "notes": []
    },
    "tools": [],
    "documents": [],
    "trade_goods": [],
    "social_cover": []
  },
  "constraints": [
    {
      "constraint_id": "constraint:cold_wet_clothes",
      "kind": "body",
      "visibility": "player_visible",
      "summary": "wet clothes make delay outside costly",
      "scene_pressure_kind": "body",
      "severity": 2
    }
  ],
  "last_changed_turn_id": "turn_0000"
}
```

## Intensity

Body/resource values use `0..5`.

| Value | Meaning |
| ---: | --- |
| 0 | absent |
| 1 | visible texture |
| 2 | relevant constraint |
| 3 | active limitation |
| 4 | severe limitation |
| 5 | crisis |

Player UI should render labels, not raw numbers, unless debug view is open.

## Body Axes

Closed V1 body axes:

- `fatigue`
- `hunger`
- `thirst`
- `cold`
- `heat`
- `pain`
- `bleeding`
- `illness`
- `poison`
- `panic`
- `sleep_debt`
- `encumbrance`

These axes feed `scene_pressure.kind=body` and sometimes `environment`,
`social_permission`, or `risk_detection`.

## Resource Types

Closed V1 resource types:

- `money`
- `food`
- `water`
- `tool`
- `weapon`
- `document`
- `clothing`
- `trade_good`
- `medicine`
- `shelter`
- `transport`
- `social_cover`
- `information_token`

`information_token` means a concrete player-visible piece of usable knowledge,
not hidden truth.

## Event Model

```json
{
  "schema_version": "singulari.body_resource_event.v1",
  "world_id": "stw_...",
  "turn_id": "turn_0002",
  "event_id": "body_resource_event_000006",
  "entity_id": "char:protagonist",
  "change": "resource_added",
  "target": "documents",
  "summary": "received a stamped entry token from the guard",
  "visibility": "player_visible",
  "mechanical_effect": {
    "gates": ["social_permission"],
    "applies_when": ["entering_west_gate"],
    "constraint": "entry_token_reduces_guard_suspicion",
    "severity": "soft_unlock"
  },
  "evidence_refs": [
    {
      "source": "visible_scene.text_blocks[5]",
      "quote": "문지기는 젖은 나무패를 내밀었다."
    }
  ],
  "created_at": "RFC3339"
}
```

Allowed changes:

- `body_axis_changed`
- `injury_added`
- `injury_changed`
- `injury_resolved`
- `resource_added`
- `resource_used`
- `resource_lost`
- `resource_revealed`
- `constraint_added`
- `constraint_resolved`

## Visibility

Body/resource visibility is strict:

| Visibility | Meaning |
| --- | --- |
| `player_visible` | the player can observe or know it |
| `inferred_visible` | fair inference from visible facts |
| `private` | engine-only internal state |
| `hidden` | secret state, never player-facing |
| `retired` | no longer active |

Hidden disease, curse, poison, or secret item knowledge may affect adjudication,
but must not appear in VN text, image prompts, or Archive View.

## Prose and Dialogue Effects

Body/resource state should alter presentation:

- fatigue shortens physical choices and slows reactions
- pain changes gestures and grip
- cold affects attention and urgency
- visible dirt/blood changes how others react
- missing document affects social permission
- scarce money changes bargaining choices

Do not explain the state every turn. Surface it when it changes, blocks a
choice, or affects another character's reaction.

## Choice Contract

Choices should respect body/resource constraints.

Examples:

- climbing with wrist pain may remain possible but costly
- entering a gate without documents needs social cover, deception, or delay
- waiting outside while cold/wet intensifies body pressure
- offering goods requires the item to exist

Freeform actions should be adjudicated against body/resource state before
relationship or lore convenience.

## Validation

Before writing state:

1. Validate entity exists.
2. Validate closed axes and resource types.
3. Validate intensity `0..5`.
4. Validate evidence refs.
5. Validate visibility does not exceed evidence.
6. Validate resource removal targets an existing resource.
7. Validate resource addition does not come from style/prose examples.
8. Validate hidden/private effects are not copied into player-visible fields.
9. Validate mechanical effects use known gates.

No fallback inventory repair invents missing items.

## Revival Selection

Each pending turn should receive:

| Bucket | Budget |
| --- | ---: |
| active body constraints | 5 |
| relevant resources | 8 |
| visible condition notes | 4 |
| hidden adjudication constraints | 2 |

Selection should prefer states touched by player input, active pressure, current
location gates, and relationship reactions.

## Implementation Plan

1. Add body/resource state and event structs.
2. Add append-only event writer and materializer.
3. Add response field `body_resource_events`.
4. Add validation and source-ref checks.
5. Add revival packet `active_body_resource_state`.
6. Feed body/resource constraints into scene pressure and choice compilation.
7. Add Archive View condition/resources section.
8. Add repair path from event log.
9. Add tests for resource existence, hidden redaction, body pressure selection,
   and freeform adjudication gates.

## Acceptance Criteria

- Physical condition and resources persist across turns.
- Choices cannot use resources that do not exist.
- Body/resource state can create scene pressure.
- Other characters can react to visible condition without hidden leakage.
- Repair can rebuild state from `body_resource_events.jsonl`.
- Malformed state changes fail preflight before partial commit.
