# Blueprint System Map

Status: architecture map

This document shows how the current implementation and proposed blueprints fit
together. The blueprints are not separate feature islands. They form one world
simulation loop:

1. persistent world state records what is true
2. revival selects what matters now
3. scene pressure compiles the current turn problem
4. WebGPT produces one structured response
5. preflight validates every projection
6. commit writes append-only evidence and materialized state
7. VN/web/MCP surfaces expose only player-visible projections

See [Causal Simulation Upgrade Set](causal-simulation-upgrade-set.md) for the
bounded implementation plan that adds a pre-turn simulation artifact,
mandatory resolution proposals, and deterministic soak tests to this map.

## Adjustment Verdict

The graph is directionally right, but it needs three cuts before
implementation.

1. Split revival into two stages.
   - `source revival` selects compact facts from durable state before scene
     pressure exists.
   - `turn context assembly` runs after pressure compilation and builds the
     final WebGPT packet.
   - This removes the circular edge where `memory_revival_policy` both selects
     pressure and depends on pressure.

2. Treat `scene_pressure` as a compiled turn artifact, not another large source
   model.
   - It should be recomputed for each pending turn from lore, relationships,
     plot threads, body/resource, location, extras, hidden timers, and player
     input.
   - Only pressure events and audit/projection state should persist.

3. Do not add every blueprint field to `AgentTurnResponse` at once.
   - The implementation should add projection families in phases.
   - Each phase adds one strict event family, validator, materializer, repair
     path, and tests.
   - A single broad response schema expansion would create a brittle mega-turn
     contract.

Keep:

- `scene_pressure` as the first implementation hub.
- append-only event logs plus materialized JSON.
- strict preflight before commit.
- separate text and image revival boundaries.

Change:

- Implement `relationship_graph` before full `character_text_design`, because
  relationship stance is needed for relation-specific speech overrides.
- Implement `memory_revival_policy` as a compiler/policy module, not as another
  source-of-truth state model.
- Move `visual_asset_graph` earlier if image ingestion/reference bugs continue
  to block testing.

## Current Implementation Spine

```mermaid
flowchart TD
  User["Player / VN web UI"] --> VnChoose["vn_server.rs<br/>POST /api/vn/choose"]
  VnChoose --> Enqueue["agent_bridge.rs<br/>enqueue_agent_turn"]
  Enqueue --> Pending["pending_agent_turn.json"]

  Pending --> Revival["revival.rs<br/>build_agent_revival_packet"]
  Revival --> WebPrompt["main.rs<br/>WebGPT prompt compiler"]
  WebPrompt --> WebGPT["WebGPT text session"]
  WebGPT --> AgentResponse["AgentTurnResponse JSON"]

  AgentResponse --> Validate["agent_bridge.rs<br/>validate_agent_response<br/>ensure_no_hidden_leak"]
  Validate --> Preflight["projection preflight<br/>extra memory / entity updates / choices"]
  Preflight --> Commit["agent_bridge.rs<br/>commit_agent_turn"]

  Commit --> Canon["canon_events.jsonl"]
  Commit --> Snapshot["latest_snapshot.json"]
  Commit --> Render["render packet / VN packet"]
  Commit --> EntityUpdates["entity_updates.jsonl<br/>relationship_updates"]
  Commit --> ExtraMemory["extra_memory.rs<br/>extra_traces.jsonl<br/>remembered_extras.json"]
  Commit --> WorldDb["world_db.rs<br/>world.db projection"]

  Render --> VnPacket["vn.rs<br/>VnPacket"]
  WorldDb --> CodexView["codex_view.rs<br/>Archive / Codex View"]
  ExtraMemory --> Revival
  EntityUpdates --> Revival
  WorldDb --> Revival
  VnPacket --> WebUi["vn-web/app.js"]
```

## Current Visual Spine

```mermaid
flowchart TD
  World["world.json + render packet"] --> Manifest["visual_assets.rs<br/>build_world_visual_assets"]
  Manifest --> Jobs["ImageGenerationJob list"]
  Jobs --> Ledger["job_ledger.rs<br/>World Jobs UI"]

  Host["host-worker / host_supervisor"] --> Claim["visual-job-claim"]
  Claim --> SessionKind["main.rs<br/>WebGptImageSessionKind validation"]
  SessionKind --> ImagePrompt["compiled image prompt"]
  ImagePrompt --> WebImage["WebGPT image session"]
  WebImage --> Ingest["probe / fetch generated output"]
  Ingest --> Complete["visual-job-complete"]

  Complete --> Assets["assets/vn/...png"]
  Complete --> Manifest
  Assets --> CurrentCg["VN current CG"]
  CurrentCg --> WebUi["VN web display"]

  Manifest --> ReferenceGate["major character design gate<br/>reference-only vs display"]
  ReferenceGate --> ImagePrompt
```

## Blueprint Layer Stack

```mermaid
flowchart BT
  Extra["Extra Memory<br/>docs/extra-memory-architecture.md"]
  Lore["World Lore<br/>docs/world-lore-blueprint.md"]
  Relations["Relationship Graph<br/>docs/relationship-graph-blueprint.md"]
  CharacterText["Character Text Design<br/>docs/character-text-design-blueprint.md"]
  Location["Location Graph<br/>docs/location-graph-blueprint.md"]
  BodyResource["Body / Resource State<br/>docs/body-resource-state-blueprint.md"]
  Plot["Plot Threads<br/>docs/plot-thread-blueprint.md"]
  Visual["Visual Asset Graph<br/>docs/visual-asset-graph-blueprint.md"]
  Pressure["Scene Pressure<br/>docs/scene-pressure-blueprint.md"]
  RevivalPolicy["Memory Revival Policy<br/>docs/memory-revival-policy-blueprint.md"]
  RetrievalController["Turn Retrieval Controller<br/>docs/working-self-memory-blueprint.md"]
  Capsules["Context Capsules<br/>docs/context-capsule-lazy-revival-blueprint.md"]

  Extra --> Relations
  Extra --> CharacterText
  Extra --> Pressure

  Lore --> Location
  Lore --> Pressure
  Lore --> RevivalPolicy

  Relations --> CharacterText
  Relations --> Pressure
  Relations --> RevivalPolicy

  CharacterText --> RevivalPolicy
  CharacterText --> Pressure

  Location --> Pressure
  Location --> Visual
  Location --> RevivalPolicy

  BodyResource --> Pressure
  BodyResource --> RevivalPolicy

  Plot --> Pressure
  Plot --> RevivalPolicy

  Visual --> RevivalPolicy
  Visual --> Pressure

  Pressure --> RevivalPolicy
  RevivalPolicy --> Capsules
  Capsules -.-> RetrievalController
  RetrievalController -.-> Capsules
  Capsules --> Pressure
```

## Proposed Unified Turn Loop

```mermaid
flowchart TD
  Input["player input"] --> Pending["PendingAgentTurn"]

  subgraph SourceState["Persistent source state"]
    WorldJson["world.json"]
    Canon["canon_events.jsonl"]
    Entities["entities.json"]
    WorldLore["world_lore.json<br/>world_lore_updates.jsonl"]
    Relationships["relationship_graph.json<br/>relationship_events.jsonl"]
    Extras["remembered_extras.json<br/>extra_traces.jsonl"]
    CharacterDesign["character_text_design.json<br/>character_text_design_events.jsonl"]
    Locations["location_graph.json<br/>location_events.jsonl"]
    BodyResources["body_resource_state.json<br/>body_resource_events.jsonl"]
    PlotThreads["plot_threads.json<br/>plot_thread_events.jsonl"]
    VisualGraph["visual_asset_graph.json<br/>visual_asset_events.jsonl"]
    Hidden["hidden_state.json"]
    PlayerKnowledge["player_knowledge.json"]
    WorldDb["world.db"]
  end

  Pending --> SourceRevival["Source Revival<br/>select compact durable facts"]

  WorldJson --> SourceRevival
  Canon --> SourceRevival
  Entities --> SourceRevival
  WorldLore --> SourceRevival
  Relationships --> SourceRevival
  Extras --> SourceRevival
  CharacterDesign --> SourceRevival
  Locations --> SourceRevival
  BodyResources --> SourceRevival
  PlotThreads --> SourceRevival
  VisualGraph --> SourceRevival
  Hidden --> SourceRevival
  PlayerKnowledge --> SourceRevival
  WorldDb --> SourceRevival

  SourceRevival --> PressureCompile["Scene Pressure Compiler<br/>what presses this turn"]
  PressureCompile --> TurnContext["Turn Context Assembly<br/>source revival + active pressure + schema budget"]
  TurnContext --> Prompt["WebGPT text prompt<br/>runtime contract + final packet + schema"]
  Prompt --> Response["AgentTurnResponse"]

  Response --> ResponseValidate["schema + hidden leak validation"]
  ResponseValidate --> ProjectionPreflight["preflight all structured updates"]

  ProjectionPreflight --> Commit["commit turn"]
  Commit --> AppendEvents["append-only events"]
  AppendEvents --> Materialize["materialize current state JSON"]
  Materialize --> RepairDb["update / repair world.db projection"]
  RepairDb --> NextPending["next pending turn context"]
```

## Write Surfaces by Blueprint

```mermaid
flowchart LR
  AgentResponse["AgentTurnResponse"] --> CanonEvent["canon_event"]

  AgentResponse --> EntityUpdate["entity_updates"]
  AgentResponse --> RelationshipEvents["relationship_events"]
  AgentResponse --> ExtraContacts["extra_contacts"]
  AgentResponse --> LoreUpdates["world_lore_updates"]
  AgentResponse --> ThreadEvents["plot_thread_events"]
  AgentResponse --> PressureEvents["scene_pressure_events"]
  AgentResponse --> CharacterTextEvents["character_text_design_events"]
  AgentResponse --> LocationEvents["location_events"]
  AgentResponse --> BodyResourceEvents["body_resource_events"]
  AgentResponse --> VisualAssetEvents["visual_asset_events"]

  CanonEvent --> CanonLog["canon_events.jsonl"]
  EntityUpdate --> Entities["entities.json<br/>entity_updates.jsonl"]
  RelationshipEvents --> RelationshipGraph["relationship_graph.json"]
  ExtraContacts --> ExtraMemory["extra_traces.jsonl<br/>remembered_extras.json"]
  LoreUpdates --> WorldLore["world_lore.json"]
  ThreadEvents --> PlotThreads["plot_threads.json"]
  PressureEvents --> PressureState["scene_pressure_state.json"]
  CharacterTextEvents --> TextDesign["character_text_design.json"]
  LocationEvents --> LocationGraph["location_graph.json"]
  BodyResourceEvents --> BodyResourceState["body_resource_state.json"]
  VisualAssetEvents --> VisualGraph["visual_asset_graph.json"]

  CanonLog --> WorldDb["world.db"]
  Entities --> WorldDb
  RelationshipGraph --> WorldDb
  ExtraMemory --> WorldDb
  WorldLore --> WorldDb
  PlotThreads --> WorldDb
  PressureState --> WorldDb
  TextDesign --> WorldDb
  LocationGraph --> WorldDb
  BodyResourceState --> WorldDb
  VisualGraph --> WorldDb
```

## Read / Revival Surfaces

```mermaid
flowchart TD
  subgraph SourceCompiler["Source Revival Compiler"]
    CandidateCollect["collect durable-state candidates"]
    SourceScore["score by input, location, recency, unresolved hooks"]
    SourceBudget["apply source budgets"]
    SourcePacket["source_revival packet"]
  end

  subgraph TurnCompiler["Turn Context Assembly"]
    PressurePacket["active_scene_pressure"]
    Visibility["split visibility sections"]
    FinalBudget["apply backend final budgets"]
    Packet["turn_context packet"]
  end

  State["world store + world.db"] --> CandidateCollect
  CandidateCollect --> SourceScore
  SourceScore --> SourceBudget
  SourceBudget --> SourcePacket
  SourcePacket --> PressurePacket
  PressurePacket --> Visibility
  SourcePacket --> Visibility
  Visibility --> FinalBudget
  FinalBudget --> Packet

  Packet --> TextBackend["WebGPT text<br/>visible + adjudication-only sections"]
  Packet --> ImageBackend["WebGPT image<br/>player-visible + accepted references only"]
  Packet --> Archive["Archive / Codex View<br/>player-visible projection"]
  Packet --> DebugUi["console status<br/>counts + reasons"]
```

## Visibility Boundary

```mermaid
flowchart LR
  Hidden["hidden/private facts"] --> Adjudication["adjudication section"]
  Hidden -.-> VisibleProse["visible_scene"]
  Hidden -.-> Archive["Archive / Codex View"]
  Hidden -.-> ImagePrompt["image prompt"]
  Hidden -.-> VisualAssets["display assets"]

  PlayerVisible["player-visible facts"] --> VisibleProse
  PlayerVisible --> Archive
  PlayerVisible --> ImagePrompt

  Inferred["inferred-visible facts"] --> VisibleProse
  Inferred --> Archive
  Inferred -.-> ImagePrompt

  AcceptedRefs["accepted visual references"] --> ImagePrompt
  AcceptedRefs --> VisualAssets
```

## Implementation Dependency Order

```mermaid
flowchart TD
  A["1. Scene Pressure V0<br/>compiled from existing state"] --> B["2. Plot Threads<br/>open-loop state"]
  A --> C["3. Body / Resource State<br/>physical constraints"]
  A --> D["4. Location Graph<br/>movement + local context"]

  B --> H["8. Turn Context Assembly<br/>source revival + pressure"]
  C --> H
  D --> H

  F["5. Relationship Graph<br/>upgrade existing relationship updates"] --> E["6. Character Text Design<br/>dialogue continuity"]
  F --> A
  E --> H

  G["7. Visual Asset Graph<br/>image/reference boundary"] --> I["9. Image backend hardening"]
  G --> H

  H --> J["10. Unified preflight batches<br/>one family at a time"]
  J --> K["11. Archive/View/console projections"]
```

Recommended practical order:

1. `scene_pressure` V0: compile from existing state without new response fields.
2. `plot_threads`: make open loops durable.
3. `body_resource_state`: add physical constraints and event validation.
4. `location_graph`: make movement and local context concrete.
5. `relationship_graph`: replace loose relationship updates with edges/events.
6. `character_text_design`: add relationship-aware speech after edges exist.
7. `visual_asset_graph`: close reference/display/generated-output boundaries.
8. `turn_context_assembly`: replace ad hoc context packing with budgets.

## Module Mapping

```mermaid
flowchart LR
  Models["models.rs<br/>core records"] --> AgentBridge["agent_bridge.rs<br/>pending/commit/validation"]
  Models --> WorldDb["world_db.rs<br/>query projection"]
  AgentBridge --> Revival["revival.rs<br/>packet compiler"]
  AgentBridge --> ExtraMemory["extra_memory.rs"]
  AgentBridge --> EntityUpdate["entity_update.rs"]

  VisualAssets["visual_assets.rs"] --> JobLedger["job_ledger.rs"]
  VisualAssets --> Vn["vn.rs"]
  Vn --> VnServer["vn_server.rs"]
  VnServer --> WebUi["vn-web/app.js/styles.css"]

  NewPressure["scene_pressure.rs<br/>proposed"] -.-> AgentBridge
  NewThreads["plot_thread.rs<br/>proposed"] -.-> AgentBridge
  NewLore["world_lore.rs<br/>proposed"] -.-> Revival
  NewRelations["relationship_graph.rs<br/>proposed"] -.-> AgentBridge
  NewTextDesign["character_text_design.rs<br/>proposed"] -.-> Revival
  NewLocation["location_graph.rs<br/>proposed"] -.-> Revival
  NewBody["body_resource.rs<br/>proposed"] -.-> AgentBridge
  NewVisualGraph["visual_asset_graph.rs<br/>proposed"] -.-> VisualAssets
  NewRevivalPolicy["memory_revival_policy.rs<br/>proposed"] -.-> Revival
```

## Closure Contract

The unified architecture should preserve the existing closed-loop rule:

- validate before writing
- append evidence before materialized projections claim state
- never complete a turn with only half the projections written
- never use fallback prose scraping to repair missing structured state
- keep hidden/private content physically separated from player-visible outputs
- make every revival item, pressure, thread, relationship, and visual asset
  traceable to source evidence
