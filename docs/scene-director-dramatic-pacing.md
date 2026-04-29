# Scene Director / Dramatic Pacing

Last updated: 2026-04-30

This document defines the next quality layer for Singulari World after
LLM-led, Rust-audited resolution.

The resolution system answers:

```text
What did the player try, and what happened?
```

The Scene Director answers:

```text
What job should this turn do for the current scene?
```

It is not a scripted plot engine. It is a pacing contract that keeps long play
from collapsing into repeated pressure descriptions, repeated hesitant prose,
or endless local escalation without scene movement.

## Problem

The current runtime can now keep world truth grounded:

- player intent is interpreted by the LLM
- resolution proposals are audited by Rust
- process ticks and actor moves can become durable events
- freeform actions can leave gate traces
- choices are grounded in affordances

That solves authority drift. It does not fully solve dramatic drift.

In browser E2E, the scene prose and choices were coherent, but the runtime still
has a predictable long-play risk:

- every turn can become another "pressure tightens" paragraph
- NPCs can keep blocking, hesitating, or implying without changing the scene job
- high tension can remain high for too many turns
- choices can be locally grounded but globally same-shaped
- scene exits can be delayed because no layer tracks when the scene has done
  enough work

The missing layer is a compact scene-level memory of dramatic function.

## Core Principle

The LLM still writes the scene.

Rust tracks the beat history and audits that each turn has a job.

```text
LLM chooses expressive beat execution.
Rust tracks pacing, repetition, scene phase, and exit pressure.
```

The Scene Director should never decide detailed plot outcomes from tables. It
should only shape the envelope:

- this turn probes
- this turn escalates
- this turn reveals
- this turn imposes cost
- this turn decompresses
- this turn transitions
- this turn ends on a concrete unresolved pressure

## Authority Split

| Surface | LLM owns | Rust owns |
| --- | --- | --- |
| Turn function | Why this beat feels natural now | Required explicit beat kind and evidence |
| Scene phase | How the prose expresses setup/escalation/reveal | Current phase, recent beat history, exit pressure |
| Tension | Sensory rhythm, dialogue distance, silence, speed | Intensity trend, anti-flatline, decompression triggers |
| Repetition | Fresh Korean phrasing and scene-specific imagery | Beat repetition budget, phrase/choice-shape warnings |
| Transition | Scene handoff prose and final image | Exit condition checks, transition event write |
| Hidden truth | May shape unrevealed pressure | No hidden motive as visible explanation |

## Scene Units

A scene is a bounded dramatic container, not a location.

The same location can contain multiple scenes:

- "blocked at the gate"
- "negotiation under observation"
- "pursuit after the gate opens"

Different locations can also belong to one scene if the dramatic question is
continuous:

- "escape before the horn sounds"

The scene unit should be defined by a question:

```text
Can the protagonist get the witness to speak before the blocker shuts it down?
```

Once that question is answered, transformed, or made obsolete, the Scene
Director should push for a transition.

## Beat Vocabulary

Use a small vocabulary. Too many beat kinds will become taxonomy noise.

```rust
enum DramaticBeatKind {
    Establish,
    Probe,
    Escalate,
    Complicate,
    Reveal,
    Cost,
    ChoicePressure,
    Decompress,
    Transition,
    Cliffhanger,
}
```

Beat meanings:

- `establish`: orient the player in current pressure and stakes
- `probe`: let the player or scene inspect a signal without resolving it
- `escalate`: increase urgency, threat, social pressure, or cost
- `complicate`: change the problem shape without simply raising intensity
- `reveal`: make a previously uncertain visible fact become player-known
- `cost`: make a resolution paid for, not free
- `choice_pressure`: sharpen the next decision into incompatible options
- `decompress`: lower immediate intensity while preserving consequence
- `transition`: move to a new scene question
- `cliffhanger`: stop at a concrete change that demands the next turn

## Scene Director Packet

Rust should compile a player-visible `SceneDirectorPacket` into the prompt
context. It should be compact and derived from existing evidence.

```rust
struct SceneDirectorPacket {
    schema_version: String,
    world_id: String,
    turn_id: String,
    current_scene: SceneArc,
    recent_beats: Vec<DramaticBeat>,
    pacing_state: PacingState,
    recommended_next_beats: Vec<DramaticBeatRecommendation>,
    forbidden_repetition: Vec<String>,
    paragraph_budget_hint: ParagraphBudgetHint,
    compiler_policy: SceneDirectorPolicy,
}
```

### SceneArc

```rust
struct SceneArc {
    scene_id: String,
    scene_question: String,
    scene_phase: ScenePhase,
    opened_turn_id: String,
    current_tension: TensionLevel,
    dominant_pressure_refs: Vec<String>,
    actor_refs: Vec<String>,
    exit_conditions: Vec<SceneExitCondition>,
    unresolved_visible_threads: Vec<String>,
    evidence_refs: Vec<String>,
}
```

Scene phases:

```rust
enum ScenePhase {
    Opening,
    Development,
    Crisis,
    Release,
    TransitionReady,
}
```

### DramaticBeat

```rust
struct DramaticBeat {
    beat_id: String,
    turn_id: String,
    beat_kind: DramaticBeatKind,
    turn_function: String,
    tension_before: TensionLevel,
    tension_after: TensionLevel,
    scene_effect: SceneEffect,
    evidence_refs: Vec<String>,
}
```

### PacingState

```rust
struct PacingState {
    recent_kind_sequence: Vec<DramaticBeatKind>,
    repeated_opening_shape_count: u8,
    repeated_closure_shape_count: u8,
    high_tension_turns: u8,
    unresolved_probe_turns: u8,
    transition_pressure: TransitionPressure,
}
```

This packet should not carry hidden truth. Hidden adjudication can influence
recommendations, but the player-visible packet must only say what can be
expressed safely.

## Advisory-First Integration

The current implementation keeps Rust as the deterministic scene authority and
lets WebGPT contribute optional structural metadata. Rust compiles
`active_scene_director` into the visible prompt context from already-visible
scene pressure, pattern debt, plot threads, actor agency, process clocks, player
intent, and recent scene-window hints. WebGPT may return a
`scene_director_proposal` beside `resolution_proposal`; if supplied, Rust audits
it before commit and materializes accepted records into the append-only scene
logs.

This keeps the loop bounded:

- no hidden data in the director packet
- no second plot authority
- optional proposal compatibility for older responses
- durable scene rhythm projection when the proposal is present

Long-play browser E2E is still needed to tune budgets, but the core compile,
audit, commit, and materialization path is implemented.

## LLM Output Shape

`AgentTurnResponse` has one optional field:

```rust
struct AgentTurnResponse {
    // existing fields...
    scene_director_proposal: Option<SceneDirectorProposal>,
}
```

Proposal:

```rust
struct SceneDirectorProposal {
    schema_version: String,
    world_id: String,
    turn_id: String,
    scene_id: String,
    beat_kind: DramaticBeatKind,
    turn_function: String,
    tension_before: TensionLevel,
    tension_after: TensionLevel,
    scene_effect: SceneEffect,
    paragraph_strategy: ParagraphStrategy,
    choice_strategy: ChoiceStrategy,
    transition: Option<SceneTransitionProposal>,
    evidence_refs: Vec<String>,
}
```

The proposal says what the turn was trying to do structurally. It does not
replace `ResolutionProposal`; it sits beside it and must use evidence visible in
the prompt context.

Example:

```json
{
  "beat_kind": "complicate",
  "turn_function": "The player sees that the witness is not merely afraid; the blocker is actively controlling who can be seen.",
  "tension_before": "high",
  "tension_after": "high",
  "scene_effect": "scene_question_narrowed",
  "paragraph_strategy": {
    "opening_shape": "observable_detail",
    "middle_shape": "blocked_relation",
    "closure_shape": "forced_next_decision"
  },
  "choice_strategy": {
    "must_change_choice_shape": true,
    "avoid_recent_choice_tags": ["본다", "묻는다"]
  },
  "evidence_refs": ["visible_scene.text_blocks[0]", "pressure:social:gate"]
}
```

## Rust Audit Contract

When WebGPT supplies a proposal, Rust validates it before commit.

Minimum checks:

1. Scene check: `scene_id` matches current scene or transition rules allow a new
   scene.
2. Evidence check: every proposal has evidence refs.
3. Beat repetition check: the same beat kind cannot dominate more than the
   configured budget without `scene_effect=necessary_stall`.
4. Tension check: high tension cannot remain unchanged for too many turns
   without cost, reveal, decompression, or transition.
5. Reveal/cost check: `reveal` and `cost` beats must be backed by
   `ResolutionProposal` outcome/effects or visible text evidence.
6. Transition check: `transition` requires an exit condition to be met,
   transformed, or explicitly blocked into a new scene question.
7. Visibility check: visible turn function, paragraph strategy, and choice
   strategy cannot expose hidden adjudication.
8. Choice-shape check: `choice_strategy` cannot ask for choices that violate
   affordance grounding.

Audit failure enters the same repairable WebGPT commit loop as resolution
failures. Omitted proposals remain compatible and do not block commits.

## Event Logs

The durable projection should use append-only logs:

```text
scene_director_events.jsonl
scene_arc_events.jsonl
```

Event record:

```rust
struct SceneDirectorEventRecord {
    schema_version: String,
    world_id: String,
    turn_id: String,
    event_id: String,
    scene_id: String,
    beat_kind: DramaticBeatKind,
    turn_function: String,
    tension_before: TensionLevel,
    tension_after: TensionLevel,
    scene_effect: SceneEffect,
    evidence_refs: Vec<String>,
    recorded_at: String,
}
```

Scene arc events:

```rust
struct SceneArcEventRecord {
    schema_version: String,
    world_id: String,
    turn_id: String,
    event_id: String,
    scene_id: String,
    event_kind: SceneArcEventKind,
    summary: String,
    to_scene_question: Option<String>,
    evidence_refs: Vec<String>,
    recorded_at: String,
}

enum SceneArcEventKind {
    SceneOpened,
    SceneQuestionUpdated,
    ExitConditionAdded,
    ExitConditionMet,
    SceneTransitioned,
    SceneClosed,
}
```

Materialized packet:

```text
scene_director.json
```

This gives prompt assembly a compact active packet and gives world.db/docs a
queryable history of scene rhythm. The materializer preserves the active
`scene_id` across fresh prompt compiles and opens a new scene id only when an
accepted transition carries a `to_scene_question`.

## Prompt Integration

WebGPT should see:

- current scene question
- current phase
- recent beat sequence
- forbidden repetition notes
- recommended next beat options
- paragraph strategy hint
- choice strategy hint

Prompt rules:

- Every turn must have one primary dramatic job.
- Do not repeat the same opening image shape more than twice.
- If tension is high for three turns, either impose cost, reveal something,
  decompress, or transition.
- A probe must eventually narrow, reveal, or fail.
- Choices should not all be "look / ask / wait" variants unless the scene
  explicitly constrains action that tightly.
- Scene transition is allowed only when the scene question has changed state.

## VN UI Integration

Do not expose a writer's-room dashboard to the player.

Useful player-visible surfaces:

- status drawer: "장면 흐름" as a short safe phrase
- text log metadata: current scene question, if already visible
- console/debug tab: full scene director packet for local QA

Examples:

```text
장면 흐름: 압박이 좁혀지는 중
장면 흐름: 단서가 행동으로 바뀌는 중
장면 흐름: 다음 장면으로 넘어갈 준비
```

Avoid:

- "beat_kind=escalate"
- "transition_pressure=high"
- "hidden reveal condition satisfied"

## Relationship To Existing Layers

| Existing layer | Scene Director uses it for |
| --- | --- |
| ResolutionProposal | Whether the beat earned success, cost, reveal, or blockage |
| ScenePressurePacket | Dominant current pressure and urgency |
| ActorAgencyPacket | Which NPC goal/move can drive the next beat |
| WorldProcessClockPacket | Whether time/process should force transition |
| PlayerIntentTracePacket | Whether the player keeps probing, resisting, avoiding, or delegating |
| PatternDebtPacket | Repeated phrasing/choice-shape warnings |
| NarrativeStyleState | How to express rhythm without importing content |

Scene Director is a coordinator. It should not duplicate these layers.

## Implementation Phases

### Phase 1: Advisory Packet And Prompt Hint

Add `src/scene_director.rs` with:

- `SceneDirectorPacket`
- `SceneArc`
- `DramaticBeat`
- `DramaticBeatRecommendation`
- `PacingState`
- `ParagraphBudgetHint`

Acceptance:

- schema serializes/deserializes
- prompt context includes `active_scene_director`
- packet is player-visible only
- WebGPT prompt treats the packet as advisory, not canon

Status: implemented as advisory baseline.

### Phase 2: Broaden Baseline Inputs

Compile a baseline `SceneDirectorPacket` from existing projections.

Inputs:

- scene pressure
- plot threads
- actor agency
- world process clock
- player intent trace
- pattern debt
- latest turn log

Acceptance:

- actor/player/process inputs influence recommendations
- packet is player-visible only
- old worlds without scene director logs still get a safe default packet

Status: implemented for prompt assembly. The baseline compiler now reads
visible plot threads, actor agency, world process clock, player intent trace,
recent scene window, scene pressure, and pattern debt. Missing materialized
scene director logs fall back to a safe advisory packet.

### Phase 3: Optional Proposal Contract

Add `scene_director_proposal` to `AgentTurnResponse` and prompt schema.

Acceptance:

- WebGPT is told every turn needs a dramatic job
- proposal is optional for compatibility
- visible text and choices still validate through existing gates

Status: implemented. `AgentTurnResponse.scene_director_proposal` is optional,
and the WebGPT schema guide describes the proposal as structural metadata beside
`resolution_proposal`.

### Phase 4: Commit-Time Audit

Audit proposals before mutation.

Acceptance:

- missing evidence fails
- repeated beat over budget fails or emits repairable critique
- transition without exit condition fails
- hidden terms cannot appear in player-visible director fields

Status: implemented for supplied proposals. Omitted proposals remain compatible;
provided proposals are audited against player-visible prompt context and
repairable WebGPT commit failures.

### Phase 5: Event Commit And Materialization

Accepted proposals append `scene_director_events.jsonl` and rebuild
`scene_director.json`.

Acceptance:

- active packet contains recent beats
- high tension counters advance
- transition pressure becomes visible to prompt context

Status: implemented as append-only projection. Accepted proposals append
`scene_director_events.jsonl` / `scene_arc_events.jsonl`, rebuild
`scene_director.json`, and expose the projection through world.db/doc/debug
surfaces. Prompt context still keeps a safe compiled fallback when no projection
exists.

### Phase 6: UI/QA Surface

Expose safe pacing status in the VN drawer and richer packet detail in the
debug/console surface.

Acceptance:

- player sees a natural "장면 흐름" phrase
- QA can inspect recent beat sequence
- no hidden/reveal-condition text leaks

Status: partially implemented. The VN status drawer now shows a safe
`장면 흐름` phrase, and runtime debug details expose the materialized
`scene_director.json` packet when present. A dedicated richer QA panel remains
future work.

### Phase 7: Long-Play Tuning

Use browser E2E and dispatch records to tune budgets.

Metrics:

- repeated beat kind count
- repeated choice tag shape count
- high tension streak length
- average turns per scene
- transition frequency
- player time from turn reveal to choice
- WebGPT turn latency

Status: scaffolding only. Materialized packets now expose repeated beat
sequence, choice-shape repetition, high-tension streak, transition pressure, and
runtime dispatch details; budget tuning still requires browser E2E sampling
over longer play.

## What To Avoid

- Do not add a hardcoded plot graph.
- Do not force every scene into a three-act structure.
- Do not make Rust choose dramatic content.
- Do not expose beat taxonomy as player-facing prose.
- Do not let "pacing" override world-state causality.
- Do not solve repetition by adding random adjectives.
- Do not add broad hidden summaries to the visible prompt.

## Success Criteria

The Scene Director is working when:

- each turn has a clear dramatic job
- choices change shape as the scene changes
- high tension either pays off, shifts, or decompresses
- NPC actions feel purposeful without becoming scripted
- scenes end before they become repetitive
- long play has rhythm: pressure, discovery, cost, release, transition
- player-visible text remains natural Korean VN prose
- Rust can explain why the scene moved without pretending to be the writer

## Summary

Resolution authority made the world trustworthy.

Scene direction should make it playable for a long time.

The target is:

```text
audited world truth
+ actor/process continuity
+ scene-level dramatic memory
+ LLM prose intelligence
```

That is the next step from "the engine remembers" to "the game has rhythm."
