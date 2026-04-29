# Character Text Design Blueprint

Status: design draft

## Problem

The simulator already has `voice_anchors`, but character speech can still drift
into generic exposition, translated Korean, or same-voice dialogue. The current
anchor surface also does not fully separate:

- how a character usually speaks
- how they behave while not speaking
- how their speech changes by relationship
- how their speech changes after canon events
- which parts are player-visible and which are hidden/private

## Goals

1. Make character text design a structured continuity surface.
2. Preserve speech, endings, tone, gestures, habits, silence, and drift.
3. Connect dialogue to relationship stance without merging the two models.
4. Let extras carry lightweight text design before promotion.
5. Keep prose style separate from character voice.
6. Prevent character design examples from becoming new scene facts.

## Non-Goals

- Do not make every character speak in a gimmick.
- Do not use famous-work imitation or few-shot prose as canon seed.
- Do not let character voice override visible scene facts or relationship state.
- Do not expose hidden motives through dialogue design.
- Do not create random dialects unless the world lore supports them.

## Proposed Surfaces

- file source: `character_text_design.json`
- append-only event source: `character_text_design_events.jsonl`
- entity projection: `entities.json.character.text_design`
- extra projection: `remembered_extras.json.text_design`
- DB projection: `character_text_designs`, `character_text_design_events`
- revival projection: `memory_revival.active_character_text_design`
- prompt section: `character_text_design_contract`

`voice_anchors` can stay as the player-visible compact projection. The new
model is the source surface that creates those anchors.

## Text Design Model

```json
{
  "schema_version": "singulari.character_text_design.v1",
  "world_id": "stw_...",
  "entity_id": "char:gate_guard",
  "visibility": "player_visible",
  "authority": "canon_event",
  "confidence": "confirmed",
  "speech": {
    "sentence_length": "short",
    "question_style": "procedural",
    "explanation_level": "low",
    "directness": "high",
    "omission_pattern": "drops reasons unless challenged"
  },
  "endings": {
    "politeness": "low_formal",
    "sentence_closers": ["다", "나"],
    "avoid": ["expository monologues", "friendly softeners"]
  },
  "tone": {
    "distance": "official_suspicion",
    "emotional_temperature": "dry",
    "lexicon": ["이름", "온 길", "소지품", "통행"],
    "taboo": ["personal sympathy before verification"]
  },
  "silence": {
    "uses_silence_when": ["checking a lie", "waiting for documents"],
    "body_signal": "looks at hands before answering"
  },
  "gestures": [
    "keeps one hand near the gate latch",
    "checks the line behind the protagonist"
  ],
  "habits": [
    "asks for identity before reacting to any story"
  ],
  "drift": [
    {
      "trigger": "identity_accepted",
      "change": "questions become shorter but less hostile"
    }
  ],
  "relationship_overrides": [
    {
      "target_entity_id": "char:protagonist",
      "when_stance": "procedural_suspicion",
      "speech_delta": {
        "question_style": "narrow_verification",
        "explanation_level": "very_low"
      }
    }
  ],
  "evidence_refs": [
    {
      "source": "visible_scene.text_blocks[4]",
      "quote": "이름. 온 길. 들고 들어갈 물건."
    }
  ],
  "created_turn_id": "turn_0001",
  "last_changed_turn_id": "turn_0001"
}
```

## Separation of Concerns

| Surface | Owns | Does Not Own |
| --- | --- | --- |
| `character_text_design` | voice, gesture, habits, silence, drift | relationship state |
| `relationship_graph` | stance between entities, trust/debt/fear | base voice |
| `world_lore` | language register and social norms | individual quirks |
| `seedless_prose_contract` | narration style | character-specific speech |
| `plot_threads` | unresolved problems | how a person speaks |

When two surfaces conflict:

1. hard canon and hidden redaction
2. world lore language/social constraints
3. relationship stance override
4. character text design
5. prose style contract

## Speech Fields

`speech` controls sentence behavior.

| Field | Meaning |
| --- | --- |
| `sentence_length` | short, mixed, long, fragmented |
| `question_style` | procedural, evasive, probing, rhetorical, naive |
| `explanation_level` | none, low, medium, high |
| `directness` | low, medium, high |
| `omission_pattern` | what the character leaves unsaid |

Avoid prose-like descriptions that cannot be checked. Prefer compact labels plus
one concrete example from evidence.

## Endings

`endings` is Korean-specific and should stay small.

Fields:

- `politeness`
- `sentence_closers`
- `softeners`
- `avoid`

Examples:

- `low_formal`: hard official endings, little warmth
- `plain_rough`: short plain speech, low social polish
- `careful_polite`: respectful but guarded
- `intimate_plain`: relaxed plain speech after trust

Do not assign dialect or archaic endings without world-lore support.

## Silence and Gesture

Dialogue quality often comes from what a character does instead of explains.

The engine should prefer:

- one physical hesitation over three lines of self-explanation
- a repeated gesture that changes meaning after events
- silence tied to relationship stance

Silence and gesture are not decorative. They should reveal pressure, distance,
or concealment.

## Drift

Drift is event-driven change.

Allowed drift triggers:

- canon event
- relationship event
- body/resource state change
- plot thread resolution/failure
- extra promotion

Drift must cite evidence. No invisible personality rewrite.

## Relationship Overrides

Relationship overrides modify delivery only when a matching edge is selected.

Examples:

- a guarded person becomes blunt with someone they trust
- a formal guard becomes clipped with a suspicious stranger
- a debtor avoids direct address around the creditor
- a frightened witness uses shorter answers near the threat

Relationship overrides never create new relationship state. They consume
`relationship_graph.stance` and `voice_effect`.

## Agent Response Contract

Extend structured response with optional text-design events:

```json
{
  "character_text_design_events": [
    {
      "entity_ref": "char:gate_guard",
      "change": "drift_added",
      "field": "speech.question_style",
      "summary": "after accepting the protagonist's name, questions narrow to origin and purpose",
      "evidence_refs": [
        {
          "source": "visible_scene.text_blocks[5]",
          "quote": "이름은 됐다. 온 길은?"
        }
      ]
    }
  ]
}
```

Text-design events are optional. They should appear when a character's voice
meaningfully changes or when a new recurring character/remembered extra needs a
stable anchor.

## Validation

Before writing text design:

1. Validate entity exists or is being promoted in the same preflight batch.
2. Validate closed enum fields.
3. Validate evidence refs.
4. Validate visibility does not exceed evidence visibility.
5. Validate no hidden motive appears in player-visible fields.
6. Validate no prose-style seed creates new character facts.
7. Validate relationship overrides reference existing or preflighted edges.
8. Validate Korean ending labels are allowed by world lore.

No fallback style guesser creates a design from prose alone.

## Revival Selection

Each pending turn should receive:

| Bucket | Budget |
| --- | ---: |
| speaking entities in current scene | all |
| nearby relevant remembered extras | 5 |
| relationship-overridden designs | 5 |
| dormant entities mentioned by input | 3 |

If a character is not likely to speak or act, include only their compact public
anchor or omit them.

## Implementation Plan

1. Add `CharacterTextDesign` and event structs.
2. Project existing `voice_anchors` into V1-compatible text design on repair.
3. Add validation and closed labels.
4. Add `character_text_design_events` to response preflight.
5. Attach selected designs to revival packet.
6. Render compact anchors in Archive View/Codex View.
7. Connect relationship `voice_effect` to text-design overrides.
8. Add tests for hidden redaction, relationship override application, Korean
   ending validation, and no-few-shot seed leakage.

## Acceptance Criteria

- Major and recurring minor characters keep distinct speech without gimmick.
- Relationship stance changes how a character addresses another entity.
- Prose style does not overwrite character voice.
- Character text design cannot introduce new lore, plot, or hidden motives.
- Remembered extras can carry compact text design before full promotion.
- Revival packet includes only characters relevant to the pending turn.
