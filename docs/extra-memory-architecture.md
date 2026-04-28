# Extra Memory Architecture

Last updated: 2026-04-28

This design makes background people feel like part of the world without turning
every passerby into a full character sheet. The simulator records contact,
promotes only meaningful extras, and retrieves a small relevant slice when a
future scene can use it.

## Goal

Even a one-scene extra may matter if the player notices them, harms them, helps
them, buys from them, lies to them, or leaves a trace near them.

The engine should remember those contacts so future turns can say:

- the same gate guard recognizes the protagonist
- a market child repeats a rumor differently after being helped
- a frightened witness avoids eye contact in a later scene
- a shopkeeper changes price or tone after a debt, insult, or favor

The goal is living continuity, not exhaustive census.

## Non-Goals

- Do not create a full `CharacterRecord` for every generated face.
- Do not retrieve every known extra every turn.
- Do not let background memory override visible canon or hidden-truth redaction.
- Do not use extras as a hidden guide, destined ally, or authorial hint channel.
- Do not write bland biography dumps into player-facing prose.

## Entity Tiers

### 1. Ephemeral Extra

An unnamed or lightly described person used only for scene texture.

Examples:

- a tired porter under the gate arch
- two wet soldiers at a roadside post
- a girl carrying turnips through the market

The engine may create them in prose without DB promotion. If they affect the
scene, record only an `extra_trace`.

### 2. Remembered Extra

An extra becomes remembered when the player meaningfully touches their world
line.

Promotion triggers:

- direct player interaction: talk, trade, threaten, help, deceive, recruit
- consequence witness: saw an injury, theft, mercy, spell, oath, crime
- resource relation: debt, bargain, gift, stolen item, paid service
- location tie: repeatedly associated with a place the player revisits
- information tie: gave, withheld, distorted, or overheard a clue
- emotional mark: fear, gratitude, resentment, curiosity, shame, admiration

Remembered extras store compact text design and contact memory, but remain
lighter than full characters.

### 3. Promoted Minor Character

A remembered extra becomes a minor character when continuity starts depending on
them.

Promotion triggers:

- appears in two or more separate turns
- has a named relationship to the protagonist or a party member
- owns a durable unresolved thread
- can change future choices, prices, access, rumors, or danger
- receives a stable name or title that the player can intentionally refer to
- has voice/tone/habit drift after visible canon events

At this point the engine writes or upgrades a normal `CharacterRecord`.

## Data Model

### Extra Trace

Append-only, cheap, and scene-local.

```json
{
  "schema_version": "singulari.extra_trace.v1",
  "world_id": "stw_...",
  "turn_id": "turn_0004",
  "trace_id": "extra_trace:turn_0004:gate_porter",
  "surface_label": "gate porter",
  "location_id": "place:west_gate",
  "scene_role": "witness",
  "contact_summary": "saw the protagonist hide a bloodied sleeve",
  "pressure_tags": ["social", "threat"],
  "promotion_hint": "witnessed risky action"
}
```

### Remembered Extra

DB-backed, but still compact.

```json
{
  "schema_version": "singulari.remembered_extra.v1",
  "world_id": "stw_...",
  "extra_id": "extra:west_gate:porter_01",
  "display_name": "the gate porter",
  "known_name": null,
  "role": "porter near the west gate",
  "home_location_id": "place:west_gate",
  "visibility": "player_visible",
  "first_seen_turn": "turn_0004",
  "last_seen_turn": "turn_0004",
  "contact_count": 1,
  "disposition": "wary",
  "memory": [
    "saw the protagonist hide a bloodied sleeve before the patrol passed"
  ],
  "text_design": {
    "speech": ["answers only after checking who is listening"],
    "endings": ["cuts sentences short around guards"],
    "tone": ["low, practical, cautious"],
    "gestures": ["wipes both hands on the rope belt before speaking"],
    "habits": ["looks toward the gatehouse before naming anyone"],
    "drift": []
  },
  "open_hooks": [
    "may report suspicious behavior if pressured by guards"
  ],
  "promotion_score": 3
}
```

### Minor Character Promotion

Promotion copies the remembered extra into `CharacterRecord` and leaves a
source link.

```json
{
  "schema_version": "singulari.extra_promotion.v1",
  "world_id": "stw_...",
  "extra_id": "extra:west_gate:porter_01",
  "character_id": "char:west_gate_porter",
  "reason": "second interaction plus active witness thread",
  "turn_id": "turn_0007"
}
```

## Retrieval Contract

Each pending text turn should receive only a small, relevant set of extra
memories. The engine should score candidates by:

| Signal | Use |
| --- | --- |
| current location | extras tied to this place or nearby routes |
| player input | names, titles, jobs, objects, factions, relationship words |
| recent turns | extras seen in the last few turns |
| active pressure | witnesses for threat/social, merchants for material, etc. |
| unresolved hooks | debts, rumors, fear, access, danger |
| relationship delta | gratitude, resentment, suspicion, obligation |

Default retrieval budget:

| Backend | Remembered extras | Extra traces | Notes |
| --- | ---: | ---: | --- |
| WebGPT text | 7 | 5 | active continuity revival |

The packet should carry evidence tiers:

```text
CharacterRecord > RememberedExtra > ExtraTrace > DerivedCandidate
```

`DerivedCandidate` may suggest possible reuse, but must not become canon unless
the turn output explicitly records it as visible contact.

## Turn Loop

```text
visible scene generated
  -> extract extra contacts from response and entity updates
  -> write extra_trace for any meaningful background contact
  -> update remembered_extra when trigger threshold is met
  -> promote to CharacterRecord when continuity depends on it
  -> next pending turn retrieves only relevant extras
```

Extraction should happen after the normal `AgentTurnResponse` schema and hidden
redaction checks pass. If extraction fails, the turn should not silently invent
fallback memories; it should leave the response committed and report the
extraction error as a repairable projection issue.

## Prompt Contract

Text backends should receive:

- `remembered_extras`: compact player-visible records selected by retrieval
- `recent_extra_traces`: very recent contacts that have not yet been promoted
- `extra_memory_policy`: rules for using and updating extras

Prompt rules:

- Use retrieved extras only when they naturally fit the current scene.
- Do not force every remembered extra to appear.
- If an extra reappears, preserve their text design: speech, endings, tone,
  gestures, habits, and drift.
- If the player changes an extra's life, record the change in memory or promote.
- Never use extra memory to leak hidden truth.

## UI / Codex View

Codex View should expose a player-visible "Local Faces" section:

- remembered extras relevant to the current location
- last contact summary
- disposition
- open player-visible hooks

It should not show promotion scores, hidden hooks, or hidden faction ties.

## Implementation Plan

1. Add `remembered_extras.json` and `extra_traces.jsonl` under each world.
2. Add Rust structs for `ExtraTrace`, `RememberedExtra`, and
   `ExtraPromotionRecord`.
3. Add a projection builder that extracts extra contacts from committed
   `AgentTurnResponse` plus structured entity/relationship updates.
4. Add retrieval into `AgentVisibleContext` with backend-specific budgets.
5. Add Codex View rendering for player-visible local faces.
6. Add repair path that rebuilds remembered extras from JSONL traces and
   promotion records.
7. Add tests for tier promotion, retrieval budget, hidden redaction, and
   no-full-character spam.

## Acceptance Criteria

- A one-off background person can leave an `extra_trace` without becoming a
  full character.
- Repeated or consequential contact promotes the person to `remembered_extra`.
- Repeated remembered contact can promote to `CharacterRecord`.
- Pending turns retrieve relevant extras by location/input/pressure without
  dumping the whole DB.
- WebGPT receives a remembered-extra window large enough for background-life
  continuity without dumping the whole DB.
- Player-visible views redact hidden hooks and never show promotion internals.
