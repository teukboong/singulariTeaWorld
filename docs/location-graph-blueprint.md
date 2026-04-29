# Location Graph Blueprint

Status: design draft

## Problem

The simulator can store place entities and world lore, but it does not yet have
a durable model for spatial continuity.

Without a location graph, movement becomes generic:

- "look around"
- "go somewhere"
- "enter the town"
- "find another route"

The world needs a map-like simulation layer that knows which places exist, which
routes connect them, what blocks access, who belongs there, what pressure lives
there, and what the player can fairly know.

## Goals

1. Make places and routes first-class simulation state.
2. Connect location to lore, relationships, extras, visual assets, and active
   pressures.
3. Let movement choices emerge from known routes and access gates.
4. Keep unknown locations hidden until discovered or inferred.
5. Preserve local continuity: faces, hazards, customs, resources, and visual
   identity.
6. Prevent WebGPT from inventing exits, towns, factions, or landmarks without
   structured evidence.

## Non-Goals

- Do not build a full grid map or tactical engine.
- Do not expose hidden locations in player-facing views.
- Do not force every world to have a complete geography up front.
- Do not let route convenience override social, resource, time, or danger gates.
- Do not create locations from atmospheric prose alone.

## Proposed Surfaces

- file source: `location_graph.json`
- append-only event source: `location_events.jsonl`
- DB projection: `locations`, `location_routes`, `location_events`
- revival projection: `memory_revival.active_location_graph`
- player projection: Archive View "Known Places"
- prompt section: `location_graph_contract`
- visual link: `visual_asset_graph.location_refs`

## Location Node

```json
{
  "schema_version": "singulari.location_node.v1",
  "world_id": "stw_...",
  "location_id": "place:west_gate",
  "name": "서쪽 문",
  "location_kind": "settlement_gate",
  "visibility": "player_visible",
  "knowledge_state": "visited",
  "summary": "해질 무렵 빗장을 거는 변경 마을의 서쪽 출입문.",
  "sensory_profile": {
    "sound": ["chain scrape", "wet boots on stone"],
    "smell": ["rain on timber", "animal sweat"],
    "light": "low evening light under the gate arch"
  },
  "local_rules": [
    "identity is requested before entry",
    "goods may be checked at closing time"
  ],
  "resident_entity_ids": ["char:gate_guard"],
  "local_extra_ids": ["extra:west_gate:porter_01"],
  "related_lore_ids": ["lore:settlement:west_gate"],
  "active_thread_ids": ["thread:enter_town_before_gate_close"],
  "active_pressure_ids": ["pressure:gate_permission"],
  "visual_asset_ids": ["ui_asset:stage_background"],
  "created_turn_id": "turn_0001",
  "last_changed_turn_id": "turn_0001"
}
```

## Route Edge

```json
{
  "schema_version": "singulari.location_route.v1",
  "world_id": "stw_...",
  "route_id": "route:road_to_west_gate",
  "from_location_id": "place:road_bend",
  "to_location_id": "place:west_gate",
  "direction": "bidirectional",
  "visibility": "player_visible",
  "knowledge_state": "known",
  "route_kind": "road",
  "travel_cost": {
    "time": "short",
    "body": "low",
    "resource": "none"
  },
  "access_gates": [
    {
      "gate": "social_permission",
      "condition": "gate_guard_allows_entry",
      "severity": "soft_block"
    }
  ],
  "danger_profile": {
    "threat": 1,
    "exposure": 2,
    "witnesses": 3
  },
  "evidence_refs": [
    {
      "source": "canon_events.evt_000001",
      "field": "visible_summary"
    }
  ]
}
```

## Location Kinds

Closed enum for V1:

- `wilderness`
- `road`
- `settlement_gate`
- `settlement_district`
- `market`
- `household`
- `worksite`
- `sacred_site`
- `ruin`
- `waterway`
- `border`
- `interior_room`
- `temporary_camp`
- `unknown`

## Knowledge State

| State | Meaning |
| --- | --- |
| `unknown` | not available to player |
| `rumored` | mentioned, uncertain |
| `inferred` | reachable conclusion from visible facts |
| `known` | known but not visited |
| `visited` | player has been there |
| `mapped` | route/constraints are clear enough for planning |
| `blocked` | known but currently inaccessible |
| `retired` | invalidated or no longer relevant |

Knowledge state controls both prompt inclusion and player-facing display.

## Route Gates

Routes should use the same mechanical axes as world lore and pressure:

- `body`
- `resource`
- `time_pressure`
- `social_permission`
- `knowledge`
- `risk_detection`
- `environment`
- `moral_cost`

Movement choices should cite route gates rather than generic "go there" text.

## Local Continuity

Each location can attach:

- resident or recurring entities
- remembered extras
- local lore
- active plot threads
- active scene pressures
- known resources
- common witnesses
- visual assets
- sensory profile

The engine should not include all local data every turn. It should select the
subset relevant to player input, active pressure, and current route options.

## Agent Response Contract

Add structured location events:

```json
{
  "location_events": [
    {
      "change": "route_discovered",
      "location_ref": "place:west_gate",
      "route_ref": "route:ditch_path_to_outer_wall",
      "summary": "The player noticed a narrow ditch path beside the wall.",
      "visibility": "player_visible",
      "knowledge_state_after": "inferred",
      "evidence_refs": [
        {
          "source": "visible_scene.text_blocks[6]",
          "quote": "벽 아래 물길이 어둡게 이어졌다."
        }
      ]
    }
  ]
}
```

Allowed changes:

- `location_discovered`
- `location_visited`
- `location_updated`
- `route_discovered`
- `route_blocked`
- `route_unblocked`
- `local_rule_added`
- `local_face_added`
- `visual_anchor_added`
- `retired`

## Movement Choice Contract

Movement choices should be generated only from:

- known route edges
- inferred route edges from visible evidence
- freeform player action adjudication
- newly proposed `location_events` in the same turn, after validation

If the player asks to go somewhere unknown, adjudication should check:

1. whether the place exists in known/inferred state
2. whether current location has a route or plausible probe action
3. which gates apply
4. whether the action opens a discovery thread instead of movement

## Validation

Before writing location state:

1. Validate ids and closed enums.
2. Validate routes reference existing or same-batch locations.
3. Validate visibility does not exceed evidence.
4. Validate hidden locations are not included in player projections.
5. Validate access gates use known mechanical axes.
6. Validate sensory profile contains observations only, not hidden lore.
7. Validate visual asset refs match asset kind and display/reference policy.
8. Validate no route is created from style text or generic genre assumption.

## Revival Selection

Each pending turn should receive:

| Bucket | Budget |
| --- | ---: |
| current location | 1 |
| immediate route options | 5 |
| nearby known locations | 3 |
| relevant local faces | 5 |
| hidden adjudication-only route constraints | 2 |

## Implementation Plan

1. Add `LocationNode`, `LocationRoute`, and event structs.
2. Add location graph materializer from append-only events and entity records.
3. Add route/access validation.
4. Add `location_events` to `AgentTurnResponse`.
5. Add revival packet section `active_location_graph`.
6. Feed known routes into choice compilation and scene pressure.
7. Link visual assets to locations.
8. Render player-visible places/routes in Archive View.
9. Add repair path that rebuilds graph from events.
10. Add tests for hidden route redaction, route gate validation, movement choice
    generation, and unknown-location adjudication.

## Acceptance Criteria

- The engine can answer where the protagonist is and where they can plausibly go.
- Movement choices come from known or inferred routes.
- Local extras, lore, pressure, and visual assets can be retrieved by location.
- Hidden places/routes do not leak into player-facing text or image prompts.
- Rebuilding from `location_events.jsonl` restores current known map state.
