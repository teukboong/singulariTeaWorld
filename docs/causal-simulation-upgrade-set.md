# Causal Simulation Upgrade Set

Last updated: 2026-04-30

Status: implementation set in progress; phases 1-2, pressure-obligation
coverage, and the offline six-scenario soak slice are implemented.

This document turns the V2 simulator direction into one bounded implementation
set:

1. `PreTurnSimulationPass`
2. mandatory non-bootstrap `ResolutionProposal`
3. scripted multi-turn simulation soak tests

The goal is to move Singulari World from "LLM writes a scene, Rust records and
audits it" toward "Rust compiles the causal world problem first, then the LLM
writes inside that bounded problem."

## Existing Baseline

The current runtime already has the important platform pieces:

- file-backed source evidence: JSON and JSONL per world
- durable SQLite projections in `world.db`
- player-visible VN packet and browser UI
- MCP tools for play, search, visual jobs, validation, and repair
- WebGPT text and image lanes as the default backend
- typed projection families for pressure, body/resources, location,
  relationships, belief, processes, consequences, encounters, and actor agency
- Rust-side validation and redaction before commit

The remaining gap is authority. The LLM can currently propose many structured
deltas, and Rust checks them, but the turn's causal affordances and obligations
are not yet compiled as one explicit pre-turn artifact before the LLM writes.

## Outcome

After this set lands, every normal player turn should have this shape:

```text
player input
-> pending turn
-> SimulationSourceBundle
-> PreTurnSimulationPass
-> prompt context with causal constraints
-> WebGPT AgentTurnResponse
-> mandatory ResolutionProposal audit
-> typed projection preflight
-> commit append-only evidence
-> materialize JSON / world.db / VN packet
-> soak harness can replay the same contract
```

If the model writes attractive prose but fails the causal contract, the turn
fails before mutation. If the player attempts an impossible action, the world
should explain a blocked, delayed, softened, or costly outcome using visible
evidence instead of silently granting the action.

## Non-Goals

- Do not replace WebGPT with a scripted narrator.
- Do not build a full physics engine.
- Do not make hidden state player-visible just to justify a result.
- Do not introduce a second player-facing chat UI.
- Do not require external services for the soak harness.
- Do not expand `AgentTurnResponse` with broad new optional fields unless a
  validator, materializer, repair path, and tests land with them.

## Core Contract

Rust owns causal admissibility. The LLM owns semantic and dramatic expression.

| Layer | Rust owns | LLM owns |
| --- | --- | --- |
| Pre-turn | Facts, affordances, blockers, due processes, pressure obligations | Nothing durable |
| Resolution | Required refs, gate audit, visibility, causality, evidence | Intent interpretation, outcome proposal, costs |
| Scene | Hidden leak checks, schema, turn identity | Korean prose, dialogue, scene-specific texture |
| Choices | Slot contract, affordance grounding, forbidden shortcuts | User-facing labels and intents |
| Persistence | Append-only events, materialized projections, repairability | Proposed typed deltas |

## Pre-Turn Simulation Pass

`PreTurnSimulationPass` is a deterministic Rust artifact compiled after
`PendingAgentTurn` exists and before the text backend receives a prompt.
The dependency direction is fixed:

```text
PendingAgentTurn + durable world state
-> SimulationSourceBundle
-> PreTurnSimulationPass
-> PromptContextPacket / WebGPT prompt
```

`PromptContextPacket` may include the pass output, but the pass must not depend
on `PromptContextPacket`. This avoids a circular compiler boundary where the
prompt context needs a pre-turn pass while the pre-turn pass needs the prompt
context.

It should not invent story. It should answer:

- What is the player trying to touch?
- Which visible affordances are available now?
- Which actions are blocked or costly under current body, resource, location,
  time, social, and knowledge constraints?
- Which visible or adjudication-only world processes may tick if this input
  spends time, changes pressure, or touches the process?
- Which pressure vector must visibly move, or which explicit no-op reason must
  be provided?
- Which state deltas are forbidden because they lack evidence or would reveal
  hidden truth?

### Proposed Type

```rust
struct PreTurnSimulationPass {
    schema_version: String,
    world_id: String,
    turn_id: String,
    player_input: String,
    input_kind: ActionInputKind,
    selected_choice: Option<SelectedChoice>,
    source_refs: Vec<SimulationSourceRef>,
    available_affordances: Vec<CompiledAffordance>,
    blocked_affordances: Vec<BlockedAffordance>,
    pressure_obligations: Vec<PressureObligation>,
    due_processes: Vec<DueProcess>,
    causal_risks: Vec<CausalRisk>,
    required_resolution_fields: RequiredResolutionFields,
    hidden_visibility_boundary: HiddenVisibilityBoundary,
}
```

### Source Inputs

The pass should compile from a lower-level `SimulationSourceBundle` built from
existing state. No new broad source of truth is required for the first
implementation.

```rust
struct SimulationSourceBundle {
    pending: PendingAgentTurn,
    affordance_graph: AffordanceGraphPacket,
    body_resource_state: BodyResourcePacket,
    location_graph: LocationGraphPacket,
    scene_pressure: ScenePressurePacket,
    world_process_clock: WorldProcessClockPacket,
    relationship_graph: RelationshipGraphPacket,
    consequence_spine: ConsequenceSpinePacket,
    encounter_surface: EncounterSurfacePacket,
    private_adjudication_context: AgentPrivateAdjudicationContext,
}
```

| Input | Use |
| --- | --- |
| `PendingAgentTurn` | player input, selected choice, turn id |
| `AffordanceGraphPacket` | ordinary slots 1-5 grounding |
| `BodyResourcePacket` | body/resource gates and costs |
| `LocationGraphPacket` | movement and spatial plausibility |
| `ScenePressurePacket` | active pressure vectors |
| `WorldProcessClockPacket` | due visible and hidden processes |
| `RelationshipGraphPacket` | social permission and stance |
| `ConsequenceSpinePacket` | returning costs and unresolved obligations |
| `EncounterSurfacePacket` | currently manipulable scene surfaces |
| `AgentPrivateAdjudicationContext` | hidden timers and unrevealed constraints |

### Output Visibility

The artifact may contain hidden/adjudication-only refs, but any field that can
reach WebGPT visible prose must be explicitly marked:

```text
player_visible
adjudication_only
forbidden_to_render
```

The prompt compiler may include adjudication-only material only inside the
adjudication context section. The visible context section must contain only
player-visible sources and visible signals.

### Persistence Policy

`PreTurnSimulationPass` is a rebuildable compiler artifact, not canonical world
truth. The default implementation should rebuild it from committed evidence
during repair. For debugging, a committed turn may store a snapshot path in the
turn commit record, but replay must not depend on that snapshot when the source
evidence can be rebuilt.

## Mandatory Resolution Proposal

For non-bootstrap agent turns, `resolution_proposal` should become required.

Allowed exceptions:

- initial world bootstrap turn before player action exists
- explicit Codex/archive query that does not mutate world time
- repair-only or validation-only tool paths
- tests that target legacy compatibility and name that compatibility

The response must fail before `advance_turn` when a required proposal is
missing.

This is fail-closed in the commit path from the first implementation. A
diagnostic CLI may offer a dry-run or warn-only report, but `host-worker`,
`vn-serve`, MCP commit tools, and normal play must not commit a normal turn that
is missing its required proposal.

### Required Resolution Coverage

For a normal player action, the proposal must include:

- interpreted intent with `target_refs` and `evidence_refs`
- at least one gate result for the touched causal dimension
- outcome kind and visible summary
- every durable effect backed by evidence
- process ticks when the pre-turn pass marks a due process as touched
- `next_choice_plan` covering slots 1-7
- slot 1-5 grounded in current affordances
- slot 6 as freeform
- slot 7 as delegated judgment

If a pre-turn pressure obligation does not move through a gate, effect, or
process tick, the proposal must state why:

```text
pressure_noop_reasons:
  - pressure_ref: "pressure:id"
    reason: "player inspected an already-known object; the pressure remains but
      the scene moved by knowledge clarification"
    evidence_refs: ["pressure:id"]
```

### Audit Additions

Extend the existing resolution audit with these checks:

1. Required proposal exists for normal turns.
2. Every `target_ref`, `pressure_ref`, `gate_ref`, `process_ref`, and
   `grounding_ref` exists in the prompt context or pre-turn pass.
3. At least one pressure obligation is satisfied or explicitly deferred with a
   visible reason.
4. Every ordinary choice plan uses a current affordance id for the same slot.
5. The proposal does not move hidden/adjudication-only processes into visible
   text before their reveal condition is satisfied.
6. Durable effects have evidence and map to an implemented projection family.
7. `next_choices` cannot expose internal refs, hidden causes, or shortcut text.

## Choice Ownership

Slots remain stable:

```text
1-5: scene-specific ordinary choices
6: freeform inline action
7: delegated judgment
```

The implementation target is:

- Rust compiles ordinary affordances and forbidden shortcuts.
- WebGPT rewrites them into polished Korean labels and intents.
- Rust audits that each label/intent stays grounded and does not reveal
  internal refs.

This prevents the model from solving the scene by inventing a convenient new
action surface.

## Process and Actor Scheduling

The first implementation should stay conservative:

- hidden timers may create visible pressure only through visible signals
- actor goals may update only from visible behavior, social exchange, or
  consequence evidence
- world processes tick only when the player action, visible time passage, or
  active pressure touches them
- offscreen progress should write adjudication-only events until a visible
  signal is emitted

Do not auto-advance every process on every turn. That creates noisy simulation
and makes repair harder.

## Soak Harness

Add a deterministic local harness that creates a temporary store, runs scripted
inputs, and validates the simulator contract after each turn.

The harness should not call WebGPT. It should use committed fixture responses
matching the current schema. Avoid a clever fixture engine; replay fixed
`AgentTurnResponse` files or keep any adapter as a thin selector from scripted
input to fixture response. The harness must not become a second narrative
backend.

### Initial Scenarios

| Scenario | Purpose |
| --- | --- |
| `sparse_medieval_male` | proves sparse seed does not inject isekai/system/hidden heroine tropes |
| `missing_resource_attempt` | proves unavailable tools/resources block or impose cost |
| `social_permission_push` | proves social outcomes are not auto-granted |
| `time_pressure_wait` | proves world process ticks only when time is spent |
| `hidden_probe` | proves hidden truth is blocked or converted to visible clue seeking |
| `route_and_return` | proves location graph and memory revival survive multiple turns |

### Per-Turn Assertions

Each scripted turn should assert:

- `validate_world` passes
- no hidden/adjudication-only text appears in `VnPacket`, Codex View, search
  snippets, or image prompts
- normal turns include a `resolution_proposal`
- ordinary choices have grounded `next_choice_plan`
- at least one pressure vector moves, or a no-op reason is recorded

### Scenario-Final Assertions

Run heavier reconstruction checks once at the end of each scenario:

- `world.db` search index rebuilds after repair
- export/import round-trip preserves visible canon and projection counts
- repeated repair does not change the reconstructed visible packet

### Command Shape

Keep this as a repo-local check, not a service:

```bash
cargo test --locked simulator_soak
```

If the suite becomes slow, split long scenarios behind an explicit script:

```bash
scripts/simulator-soak.sh
```

## Implementation Phases

### Phase 1: Artifact and Prompt Boundary

Implementation status: done.

- Add `pre_turn_simulation.rs`.
- Compile `SimulationSourceBundle` from existing durable world state and the
  pending turn.
- Compile `PreTurnSimulationPass` from `SimulationSourceBundle`.
- Add the artifact to `PromptContextPacket` or adjacent prompt assembly after
  the pass is built.
- Ensure visible/adjudication-only sections stay separate.
- Add unit tests for hidden boundary and sparse seed behavior.

Acceptance:

```text
cargo test --locked pre_turn_simulation
cargo test --locked prompt_context
```

### Phase 2: Mandatory Resolution Gate

Implementation status: done for the current set. Mandatory proposal, pressure
coverage, explicit pressure no-op reasons, and slot plan audit are implemented.

- Compile the required proposal decision from `PreTurnSimulationPass`.
- Enforce it before `advance_turn`.
- Extend `audit_resolution_proposal` to consume the pre-turn pass.
- Require complete `next_choice_plan` for normal turns.
- Add tests for missing proposal, unknown refs, unfulfilled pressure, and slot
  grounding.

Acceptance:

```text
cargo test --locked resolution
cargo test --locked agent_bridge
```

### Phase 3: Choice Grounding Tightening

Implementation status: partially done through resolution/choice audit.

- Make slots 1-5 originate from compiled affordances.
- Preserve WebGPT ownership of labels and intent wording.
- Reject labels/intents that expose internal ids, hidden causes, or forbidden
  shortcuts.
- Keep slot 6 freeform and slot 7 delegated judgment compatibility.

Acceptance:

```text
cargo test --locked affordance_graph
cargo test --locked vn
```

### Phase 4: Soak Harness

Implementation status: deterministic six-scenario fixture replay plus
scenario-final repair/export/import checks landed.

- Add a fixture replay adapter that only maps scripted inputs to fixed response
  files or fixed in-test response objects.
- Add 6 initial scripted scenarios.
- Run scenario-final checks for world.db search, repair, and export/import.
- Keep the harness deterministic and offline.

Acceptance:

```text
cargo test --locked simulator_soak
scripts/smoke-local.sh
```

### Phase 5: Release Gate

- Run full local release checks.
- Add the new harness to `scripts/release-build.sh` only if it stays fast and
  deterministic.
- Otherwise document it as an explicit pre-alpha confidence check.

Acceptance:

```text
scripts/privacy-audit.sh
cargo fmt --all -- --check
cargo check --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
```

## Failure Modes

| Failure | Expected behavior |
| --- | --- |
| Missing mandatory resolution proposal | fail before mutation |
| Unknown target ref | structured critique, no commit |
| Hidden process leaked into visible scene | reject response |
| No pressure movement and no no-op reason | reject response |
| Choice plan not grounded in affordance | reject response |
| Fixture replay adapter emits stale schema | soak test fails |
| Repair changes visible packet | soak test fails |

## Migration Notes

- Existing worlds remain readable.
- Bootstrap turns may keep weaker contracts until a player action exists.
- Legacy guide-choice wording stays redacted at render time.
- Current slot contract is `6 = 자유서술`, `7 = 판단 위임`; do not reintroduce
  older slot-4 guide semantics.
- Public docs should continue to describe WebGPT as the default backend, but the
  causal contract should be backend-agnostic.

## Open Questions

1. Should debug snapshots of `PreTurnSimulationPass` be written beside commit
   records, or only emitted by diagnostic commands?
2. Should the fixture replay adapter live under tests only, or behind a
   diagnostic CLI command?
3. How much adjudication-only process state should WebGPT see when it is not
   needed for the current input?

The default answer for implementation should be conservative: keep the artifact
rebuildable, fail closed at commit, and expose only player-visible signals to
rendered surfaces.
