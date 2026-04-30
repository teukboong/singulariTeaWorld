#![recursion_limit = "256"]

mod actor_agency;
mod adjudication;
mod affordance_graph;
mod agent_bridge;
mod autobiographical_index;
mod backend_selection;
mod belief_graph;
mod body_resource;
mod change_ledger;
mod character_text_design;
mod chat;
mod codex_view;
mod consequence_spine;
mod context_capsule;
mod encounter_surface;
mod entity_update;
mod event_ledger;
mod extra_memory;
mod host_supervisor;
mod job_ledger;
mod knowledge_ledger;
mod location_graph;
mod memory_revival;
mod memory_revival_policy;
mod models;
mod narrative_style_state;
mod pattern_debt;
mod player_intent;
mod plot_thread;
mod pre_turn_simulation;
mod projection_health;
mod projection_registry;
mod prompt_context;
mod prompt_context_budget;
mod relationship_graph;
mod render;
mod resolution;
mod response_context;
mod resume;
mod revival;
mod runtime_profile;
mod scene_director;
mod scene_pressure;
#[cfg(test)]
mod simulator_soak;
mod social_exchange;
pub mod sqlite;
mod start;
mod store;
mod transfer;
mod turn;
mod turn_commit;
mod turn_context;
mod turn_retrieval_controller;
pub mod validate;
mod visual_asset_graph;
mod visual_assets;
mod vn;
mod vn_server;
mod voice_anchor;
mod world_court;
mod world_db;
mod world_docs;
mod world_lore;
mod world_process_clock;

pub use actor_agency::{
    ACTOR_AGENCY_FILENAME, ACTOR_AGENCY_PACKET_SCHEMA_VERSION, ACTOR_GOAL_EVENT_SCHEMA_VERSION,
    ACTOR_GOAL_EVENTS_FILENAME, ACTOR_GOAL_SCHEMA_VERSION, ACTOR_MOVE_EVENT_SCHEMA_VERSION,
    ACTOR_MOVE_EVENTS_FILENAME, ACTOR_MOVE_SCHEMA_VERSION, ActorAgencyEventPlan, ActorAgencyPacket,
    ActorAgencyPolicy, ActorGoal, ActorGoalEventRecord, ActorMove, ActorMoveEventRecord,
    AgentActorGoalUpdate, AgentActorMoveUpdate, append_actor_agency_event_plan,
    build_actor_agency_from_events, load_actor_agency_state, merge_consequence_actor_agency,
    merge_social_exchange_actor_agency, prepare_actor_agency_event_plan,
    rebuild_actor_agency_packet,
};
pub use adjudication::{AdjudicationInput, adjudicate_turn};
pub use affordance_graph::{
    AFFORDANCE_GRAPH_PACKET_SCHEMA_VERSION, AFFORDANCE_NODE_SCHEMA_VERSION, AffordanceGraphPacket,
    AffordanceGraphPolicy, AffordanceKind, AffordanceNode, ORDINARY_AFFORDANCE_SLOT_COUNT,
    compile_affordance_graph_packet, compile_affordance_graph_packet_with_encounter,
};
pub use agent_bridge::{
    AGENT_COMMIT_RECORD_SCHEMA_VERSION, AGENT_PENDING_TURN_SCHEMA_VERSION,
    AGENT_TURN_RESPONSE_SCHEMA_VERSION, AgentCommitTurnOptions, AgentExtraContact,
    AgentHiddenSecret, AgentHiddenTimer, AgentOutputContract, AgentPrivateAdjudicationContext,
    AgentResponseAdjudication, AgentResponseCanonEvent, AgentSubmitTurnOptions, AgentTurnResponse,
    AgentVisibleContext, AgentVoiceAnchor, CommittedAgentTurn, PendingAgentChoice,
    PendingAgentTurn, commit_agent_turn, enqueue_agent_turn, load_pending_agent_turn,
};
pub use autobiographical_index::{
    AUTOBIOGRAPHICAL_GENERAL_EVENT_SCHEMA_VERSION, AUTOBIOGRAPHICAL_INDEX_FILENAME,
    AUTOBIOGRAPHICAL_INDEX_SCHEMA_VERSION, AUTOBIOGRAPHICAL_PERIOD_SCHEMA_VERSION,
    AutobiographicalGeneralEvent, AutobiographicalGeneralEventKind, AutobiographicalIndexInput,
    AutobiographicalIndexPacket, AutobiographicalIndexPolicy, AutobiographicalPeriod,
    load_autobiographical_index_state, rebuild_autobiographical_index,
};
pub use backend_selection::{
    WORLD_BACKEND_SELECTION_FILENAME, WORLD_BACKEND_SELECTION_SCHEMA_VERSION,
    WorldBackendSelection, WorldTextBackend, WorldVisualBackend, backend_selection_path,
    load_world_backend_selection, save_world_backend_selection,
};
pub use belief_graph::{
    BELIEF_EVENT_SCHEMA_VERSION, BELIEF_EVENTS_FILENAME, BELIEF_GRAPH_FILENAME,
    BELIEF_GRAPH_PACKET_SCHEMA_VERSION, BELIEF_NODE_SCHEMA_VERSION, BeliefConfidence,
    BeliefEventPlan, BeliefEventRecord, BeliefGraphPacket, BeliefGraphPolicy, BeliefHolder,
    BeliefNode, append_belief_event_plan, compile_belief_graph_packet, load_belief_graph_state,
    prepare_belief_event_plan, rebuild_belief_graph,
};
pub use body_resource::{
    BODY_CONSTRAINT_SCHEMA_VERSION, BODY_RESOURCE_EVENT_SCHEMA_VERSION,
    BODY_RESOURCE_EVENTS_FILENAME, BODY_RESOURCE_PACKET_SCHEMA_VERSION,
    BODY_RESOURCE_STATE_FILENAME, BodyConstraint, BodyResourceEvent, BodyResourceEventKind,
    BodyResourceEventPlan, BodyResourceEventRecord, BodyResourcePacket, BodyResourcePolicy,
    BodyResourceVisibility, RESOURCE_ITEM_SCHEMA_VERSION, ResourceItem, ResourceKind,
    append_body_resource_event_plan, compile_body_resource_packet, load_body_resource_state,
    prepare_body_resource_event_plan, rebuild_body_resource_state,
};
pub use change_ledger::{
    CHANGE_EVENT_SCHEMA_VERSION, CHANGE_EVENTS_FILENAME, CHANGE_LEDGER_FILENAME,
    CHANGE_LEDGER_SCHEMA_VERSION, ChangeAxis, ChangeEventPlan, ChangeEventPlanInput,
    ChangeEventRecord, ChangeLedgerPacket, ChangeLedgerPolicy, append_change_event_plan,
    load_change_ledger_state, prepare_change_event_plan, rebuild_change_ledger,
};
pub use character_text_design::{
    CHARACTER_TEXT_DESIGN_EVENT_SCHEMA_VERSION, CHARACTER_TEXT_DESIGN_EVENTS_FILENAME,
    CHARACTER_TEXT_DESIGN_FILENAME, CHARACTER_TEXT_DESIGN_PACKET_SCHEMA_VERSION,
    CHARACTER_TEXT_DESIGN_SCHEMA_VERSION, CharacterTextDesign, CharacterTextDesignEventPlan,
    CharacterTextDesignEventRecord, CharacterTextDesignPacket, CharacterTextDesignPolicy,
    append_character_text_design_event_plan, build_character_text_design_from_events,
    compile_character_text_design_packet, compile_character_text_design_with_projection,
    load_character_text_design_event_records, load_character_text_design_state,
    prepare_character_text_design_event_plan, rebuild_character_text_design,
};
pub use chat::{
    CHAT_ROUTE_SCHEMA_VERSION, ChatRoute, ChatRouteOptions, render_chat_route, route_chat_input,
};
pub use codex_view::{
    BuildCodexViewOptions, CodexViewSection, build_codex_view, render_codex_view_markdown,
    render_codex_view_section_markdown,
};
pub use consequence_spine::{
    ACTIVE_CONSEQUENCES_FILENAME, ActiveConsequence, CONSEQUENCE_EVENT_SCHEMA_VERSION,
    CONSEQUENCE_EVENTS_FILENAME, CONSEQUENCE_PROPOSAL_SCHEMA_VERSION,
    CONSEQUENCE_RETURN_RIGHTS_SCHEMA_VERSION, CONSEQUENCE_SCHEMA_VERSION,
    CONSEQUENCE_SPINE_PACKET_SCHEMA_VERSION, ConsequenceDecay, ConsequenceEventKind,
    ConsequenceEventPlan, ConsequenceEventRecord, ConsequenceFollowup, ConsequenceKind,
    ConsequenceMemory, ConsequenceMutation, ConsequencePayoff, ConsequencePressureLink,
    ConsequenceProposal, ConsequenceReturnRights, ConsequenceReturnTrigger,
    ConsequenceReturnWindow, ConsequenceScope, ConsequenceSeverity, ConsequenceSpinePacket,
    ConsequenceSpinePolicy, ConsequenceStatus, EphemeralConsequenceReason,
    append_consequence_event_plan, audit_consequence_contract, consequence_can_drive_world_process,
    consequence_can_return_as_scene_pressure, load_consequence_spine_state,
    prepare_consequence_event_plan, rebuild_consequence_spine,
};
pub use context_capsule::{
    CONTEXT_CAPSULE_DIR, CONTEXT_CAPSULE_INDEX_ENTRY_SCHEMA_VERSION,
    CONTEXT_CAPSULE_INDEX_FILENAME, CONTEXT_CAPSULE_INDEX_SCHEMA_VERSION,
    CONTEXT_CAPSULE_SCHEMA_VERSION, CONTEXT_CAPSULE_SELECTION_EVENT_SCHEMA_VERSION,
    CONTEXT_CAPSULE_SELECTION_EVENTS_FILENAME, CONTEXT_CAPSULE_SELECTION_SCHEMA_VERSION,
    ContextCapsule, ContextCapsuleBudgetReport, ContextCapsuleBuildInput,
    ContextCapsuleEvidenceRef, ContextCapsuleIndex, ContextCapsuleIndexEntry, ContextCapsuleKind,
    ContextCapsulePolicy, ContextCapsuleRejectReason, ContextCapsuleSelection,
    ContextCapsuleSelectionEvent, ContextCapsuleSelectionInput, ContextCapsuleSelectionReason,
    ContextCapsuleVisibility, RejectedContextCapsule, SelectedContextCapsule,
    rebuild_context_capsule_registry, select_context_capsules,
};
pub use encounter_surface::{
    AffordanceAvailability, BlockedInteraction, CHOICE_CONTRACT_SCHEMA_VERSION, ChoiceContract,
    ChoiceTimeCost, ENCOUNTER_AFFORDANCE_SCHEMA_VERSION, ENCOUNTER_CHANGE_POTENTIAL_SCHEMA_VERSION,
    ENCOUNTER_CONSTRAINT_SCHEMA_VERSION, ENCOUNTER_PROPOSAL_SCHEMA_VERSION,
    ENCOUNTER_SURFACE_EVENT_SCHEMA_VERSION, ENCOUNTER_SURFACE_EVENTS_FILENAME,
    ENCOUNTER_SURFACE_FILENAME, ENCOUNTER_SURFACE_PACKET_SCHEMA_VERSION,
    ENCOUNTER_SURFACE_SCHEMA_VERSION, EncounterActionKind, EncounterAffordance,
    EncounterChangeKind, EncounterChangePotential, EncounterClosureKind, EncounterConstraint,
    EncounterConstraintKind, EncounterPersistence, EncounterProposal, EncounterSalience,
    EncounterSurface, EncounterSurfaceChange, EncounterSurfaceClosure, EncounterSurfaceEventPlan,
    EncounterSurfaceEventRecord, EncounterSurfaceKind, EncounterSurfaceLifecycle,
    EncounterSurfaceMutation, EncounterSurfacePacket, EncounterSurfacePolicy,
    EncounterSurfaceStatus, append_encounter_surface_event_plan, compile_encounter_surface_packet,
    derive_resolution_gate_mutations, encounter_status, load_encounter_surface_state,
    prepare_encounter_surface_event_plan, rebuild_encounter_surface,
};
pub use entity_update::{EntityUpdateInput, apply_structured_entity_updates};
pub use event_ledger::{
    WORLD_EVENT_HASH_ALGORITHM, WORLD_EVENT_HASH_VERSION, WORLD_EVENT_LEDGER_SCHEMA_VERSION,
    WorldEventLedgerAppendReport, WorldEventLedgerChainReport, WorldEventLedgerChainStatus,
    WorldEventLedgerVerificationReport, verify_world_event_ledger,
};
pub use extra_memory::{
    EXTRA_MEMORY_PROJECTION_RECORD_SCHEMA_VERSION, EXTRA_MEMORY_PROJECTIONS_FILENAME,
    EXTRA_TRACE_SCHEMA_VERSION, EXTRA_TRACES_FILENAME, ExtraMemoryPacket, ExtraMemoryPolicy,
    ExtraMemoryProjectionRecord, ExtraMemoryProjectionStatus, ExtraMemoryRepairReport,
    ExtraMemoryRetrievalBudget, ExtraTrace, LocalFaceEntry, REMEMBERED_EXTRA_SCHEMA_VERSION,
    REMEMBERED_EXTRAS_FILENAME, REMEMBERED_EXTRAS_SCHEMA_VERSION, RememberedExtra,
    RememberedExtrasStore, apply_extra_memory_projection, commit_extra_memory_projection,
    commit_extra_memory_projection_terminal, compile_extra_memory_projection,
    failed_projection_records_after_latest_repair, load_extra_memory_projection_records,
    local_faces_for_codex_view, repair_extra_memory_projection, retrieve_extra_memory_packet,
};
pub use host_supervisor::{
    HOST_SUPERVISOR_PLAN_SCHEMA_VERSION, HostSupervisorLaneKind, HostSupervisorLanePlan,
    HostSupervisorPlan, HostSupervisorStatus, build_host_supervisor_plan,
    render_host_supervisor_plan,
};
pub use job_ledger::{
    ReadWorldJobsOptions, WORLD_JOB_LEDGER_SCHEMA_VERSION, WorldJob, WorldJobKind, WorldJobStatus,
    WriteTextTurnJobOptions, WriteVisualJobOptions, read_world_jobs, write_text_turn_job,
    write_visual_job,
};
pub use knowledge_ledger::{
    KNOWLEDGE_CLAIM_SCHEMA_VERSION, KnowledgeClaim, KnowledgeTier, PlayerRenderPermission,
    TruthStatus, can_render_knowledge_tier_to_player, player_render_permission,
    render_rule_for_player,
};
pub use location_graph::{
    LOCATION_EVENT_SCHEMA_VERSION, LOCATION_EVENTS_FILENAME, LOCATION_GRAPH_FILENAME,
    LOCATION_GRAPH_PACKET_SCHEMA_VERSION, LOCATION_NODE_SCHEMA_VERSION, LocationEvent,
    LocationEventKind, LocationEventPlan, LocationEventRecord, LocationGraphPacket,
    LocationGraphPolicy, LocationKnowledgeState, LocationNode, append_location_event_plan,
    compile_location_graph_packet, load_location_graph_state, prepare_location_event_plan,
    rebuild_location_graph,
};
pub use memory_revival::{
    MEMORY_REVIVAL_EVENT_SCHEMA_VERSION, MEMORY_REVIVAL_EVENTS_FILENAME,
    MEMORY_REVIVAL_ITEM_SCHEMA_VERSION, MemoryRevivalCompileInput, MemoryRevivalEvent,
    MemoryRevivalEvidenceRef, MemoryRevivalItem, MemoryRevivalReason, MemoryRevivalRejectReason,
    MemoryRevivalSelection, MemoryRevivalSourceKind, compile_memory_revival_selection,
    load_memory_revival_events,
};
pub use memory_revival_policy::{MEMORY_REVIVAL_POLICY_SCHEMA_VERSION, MemoryRevivalPolicy};
pub use models::{
    ADJUDICATION_SCHEMA_VERSION, ANCHOR_CHARACTER_ID, ANCHOR_CHARACTER_INVARIANT, AdjudicationGate,
    AdjudicationReport, AnchorCharacter, CANON_EVENT_SCHEMA_VERSION, CODEX_VIEW_SCHEMA_VERSION,
    CanonEvent, CharacterBody, CharacterRecord, CharacterVoiceAnchor, CodexAnalysisEntry,
    CodexEntityEntry, CodexFactEntry, CodexHiddenFilter, CodexRecommendation, CodexTimelineEntry,
    CodexView, CodexVoiceAnchorEntry, CurrentEvent, DEFAULT_CHOICE_COUNT, DashboardSummary,
    ENTITY_UPDATE_SCHEMA_VERSION, EntityName, EntityRecords, EntityUpdateRecord, EventAuthority,
    FREEFORM_CHOICE_SLOT, FREEFORM_CHOICE_TAG, GUIDE_CHOICE_REDACTED_INTENT, GUIDE_CHOICE_SLOT,
    HiddenState, HiddenStateSecret, HiddenStateTimer, NARRATIVE_SCENE_SCHEMA_VERSION,
    NarrativeScene, PlayerKnowledge, ProtagonistState, RENDER_PACKET_SCHEMA_VERSION,
    RelationshipUpdateRecord, RenderPacket, SINGULARI_WORLD_SCHEMA_VERSION, ScanTarget,
    StructuredEntityUpdates, TURN_LOG_ENTRY_SCHEMA_VERSION, TURN_SNAPSHOT_SCHEMA_VERSION,
    TurnChoice, TurnInputKind, TurnLogEntry, TurnSnapshot, VisibleState, WorldEventKind,
    WorldPremise, WorldRecord, WorldSeed, default_freeform_choice, default_turn_choices,
    normalize_turn_choices,
};
pub use narrative_style_state::{
    NARRATIVE_STYLE_EVENT_SCHEMA_VERSION, NARRATIVE_STYLE_EVENTS_FILENAME,
    NARRATIVE_STYLE_STATE_FILENAME, NARRATIVE_STYLE_STATE_SCHEMA_VERSION, NarrativeStyleEventPlan,
    NarrativeStyleEventRecord, NarrativeStylePolicy, NarrativeStyleState, StyleVector,
    append_narrative_style_event_plan, compile_narrative_style_state, load_narrative_style_state,
    prepare_narrative_style_event_plan, rebuild_narrative_style_state,
};
pub use pattern_debt::{
    PATTERN_DEBT_EVENT_SCHEMA_VERSION, PATTERN_DEBT_EVENTS_FILENAME, PATTERN_DEBT_FILENAME,
    PATTERN_DEBT_PACKET_SCHEMA_VERSION, PatternDebtEventPlan, PatternDebtPacket, PatternDebtPolicy,
    PatternDebtRecord, PatternSurface, append_pattern_debt_event_plan, load_pattern_debt_state,
    prepare_pattern_debt_event_plan, rebuild_pattern_debt,
};
pub use player_intent::{
    PLAYER_INTENT_EVENT_SCHEMA_VERSION, PLAYER_INTENT_EVENTS_FILENAME,
    PLAYER_INTENT_TRACE_FILENAME, PLAYER_INTENT_TRACE_SCHEMA_VERSION, PlayerIntentEventPlan,
    PlayerIntentEventRecord, PlayerIntentPolicy, PlayerIntentTracePacket,
    append_player_intent_event_plan, load_player_intent_trace_state,
    prepare_player_intent_event_plan, rebuild_player_intent_trace,
};
pub use plot_thread::{
    PLOT_THREAD_AUDIT_FILENAME, PLOT_THREAD_AUDIT_SCHEMA_VERSION, PLOT_THREAD_EVENT_SCHEMA_VERSION,
    PLOT_THREAD_EVENTS_FILENAME, PLOT_THREAD_PACKET_SCHEMA_VERSION, PLOT_THREAD_SCHEMA_VERSION,
    PLOT_THREADS_FILENAME, PlotThread, PlotThreadAuditRecord, PlotThreadChange, PlotThreadEvent,
    PlotThreadEventPlan, PlotThreadEventRecord, PlotThreadKind, PlotThreadPacket, PlotThreadPolicy,
    PlotThreadStatus, PlotThreadUrgency, append_plot_thread_audit, append_plot_thread_event_plan,
    compile_plot_thread_packet, load_plot_threads, prepare_plot_thread_event_plan,
    rebuild_plot_threads,
};
pub use pre_turn_simulation::{
    PRE_TURN_SIMULATION_PASS_SCHEMA_VERSION, PreTurnSimulationPass,
    SIMULATION_SOURCE_BUNDLE_SCHEMA_VERSION, SimulationSourceBundle,
    compile_pre_turn_simulation_pass,
};
pub use projection_health::{
    PROJECTION_HEALTH_SCHEMA_VERSION, ProjectionComponentHealth, ProjectionHealthReport,
    ProjectionHealthStatus, build_projection_health_report, render_projection_health_report,
};
pub use projection_registry::{
    BodyResourceProjectionFamily, PROJECTION_FAMILY_REGISTRY, ProjectionFamily,
    ProjectionFamilyDescriptor, SnapshotProjectionFamily, load_body_resource_prompt_packet,
};
pub use prompt_context::{
    CompilePromptContextPacketOptions, PROMPT_CONTEXT_PACKET_SCHEMA_VERSION,
    PromptAdjudicationContext, PromptContextPacket, PromptContextPolicy, PromptVisibleContext,
    assemble_prompt_context_packet, compile_prompt_context_packet,
    extract_prompt_context_from_prompt,
};
pub use prompt_context_budget::{
    PROMPT_CONTEXT_BUDGET_REPORT_SCHEMA_VERSION, PromptContextBudgetBucket,
    PromptContextBudgetExclusion, PromptContextBudgetInclusion, PromptContextBudgetPolicy,
    PromptContextBudgetReport, PromptContextExclusionReason, PromptContextInclusionReason,
    compile_prompt_context_budget_report,
};
pub use relationship_graph::{
    RELATIONSHIP_EDGE_SCHEMA_VERSION, RELATIONSHIP_GRAPH_EVENT_SCHEMA_VERSION,
    RELATIONSHIP_GRAPH_EVENTS_FILENAME, RELATIONSHIP_GRAPH_FILENAME,
    RELATIONSHIP_GRAPH_PACKET_SCHEMA_VERSION, RelationshipEdge, RelationshipGraphEventPlan,
    RelationshipGraphEventRecord, RelationshipGraphPacket, RelationshipGraphPolicy,
    append_relationship_graph_event_plan, build_relationship_graph_from_events,
    compile_relationship_graph_from_projection, compile_relationship_graph_packet,
    load_relationship_graph_event_records, load_relationship_graph_state,
    prepare_relationship_graph_event_plan, rebuild_relationship_graph,
};
pub use render::{RenderPacketLoadOptions, load_render_packet, render_packet_markdown};
pub use resolution::{
    ActionAmbiguity, ActionInputKind, ActionIntent, ChoicePlan, ChoicePlanKind,
    FREEFORM_GATE_TRACE_SCHEMA_VERSION, FreeformGateTrace, GateKind, GateResult, GateStatus,
    NarrativeBrief, ProcessTickCause, ProcessTickProposal, ProposedEffect, ProposedEffectKind,
    RESOLUTION_PROPOSAL_SCHEMA_VERSION, ResolutionCritique, ResolutionFailureKind,
    ResolutionOutcome, ResolutionOutcomeKind, ResolutionProposal, ResolutionVisibility,
    audit_resolution_choices, audit_resolution_proposal, freeform_gate_trace_from_proposal,
};
pub use response_context::{
    AGENT_CONTEXT_EVENT_SCHEMA_VERSION, AGENT_CONTEXT_EVENTS_FILENAME,
    AGENT_CONTEXT_PROJECTION_FILENAME, AGENT_CONTEXT_PROJECTION_SCHEMA_VERSION,
    AgentCharacterTextDesignUpdate, AgentContextEventInput, AgentContextEventPlan,
    AgentContextEventRecord, AgentContextProjection, AgentContextProjectionCounts,
    AgentContextProjectionItem, AgentEntityUpdate, AgentHiddenStateDelta, AgentRelationshipUpdate,
    AgentWorldLoreUpdate, ContextEventKind, ContextVisibility, HiddenDeltaKind,
    append_agent_context_event_plan, build_agent_context_projection,
    load_agent_context_event_records, load_agent_context_projection,
    prepare_agent_context_event_plan, rebuild_agent_context_projection,
};
pub use resume::{
    BuildResumePackOptions, HiddenStateSummary, RESUME_PACK_SCHEMA_VERSION, ResumePack,
    build_resume_pack, render_resume_pack_markdown,
};
pub use revival::{
    AGENT_REVIVAL_PACKET_SCHEMA_VERSION, AgentRevivalCompileOptions, build_agent_revival_packet,
};
pub use runtime_profile::{
    RUNTIME_CAPABILITY_PROFILE_SCHEMA_VERSION, RuntimeCapability, RuntimeCapabilityBoundary,
    RuntimeCapabilityProfile, RuntimeSurfaceKind,
};
pub use scene_director::{
    ChoiceStrategy, DramaticBeat, DramaticBeatKind, DramaticBeatRecommendation,
    ParagraphBudgetHint, ParagraphStrategy, SCENE_DIRECTOR_PACKET_SCHEMA_VERSION,
    SCENE_DIRECTOR_PROPOSAL_SCHEMA_VERSION, SceneArc, SceneDirectorCompileInput,
    SceneDirectorCritique, SceneDirectorFailureKind, SceneDirectorPacket, SceneDirectorPolicy,
    SceneDirectorProposal, SceneDirectorTuningMetrics, SceneEffect, SceneExitCondition, ScenePhase,
    SceneTransitionProposal, TensionLevel, TransitionPressure, audit_scene_director_proposal,
    compile_scene_director_packet, compile_scene_director_packet_from_input,
    merge_scene_director_history,
};
pub use scene_pressure::{
    ACTIVE_SCENE_PRESSURES_FILENAME, SCENE_PRESSURE_AUDIT_FILENAME,
    SCENE_PRESSURE_AUDIT_SCHEMA_VERSION, SCENE_PRESSURE_EVENT_SCHEMA_VERSION,
    SCENE_PRESSURE_EVENTS_FILENAME, SCENE_PRESSURE_PACKET_SCHEMA_VERSION,
    SCENE_PRESSURE_SCHEMA_VERSION, ScenePressure, ScenePressureAuditRecord, ScenePressureChange,
    ScenePressureEvent, ScenePressureEventPlan, ScenePressureEventRecord, ScenePressureKind,
    ScenePressurePacket, ScenePressurePolicy, ScenePressureProseEffect, ScenePressureUrgency,
    ScenePressureVisibility, append_scene_pressure_audit, append_scene_pressure_event_plan,
    compile_scene_pressure_packet, load_active_scene_pressures, merge_consequence_scene_pressures,
    merge_social_exchange_scene_pressures, prepare_scene_pressure_event_plan,
    rebuild_active_scene_pressures,
};
pub use social_exchange::{
    AskStatus, CONVERSATION_LEVERAGE_SCHEMA_VERSION, ConversationLeverage,
    ConversationLeverageKind, ConversationLeverageMutation, DIALOGUE_STANCE_FILENAME,
    DialogueStance, DialogueStanceKind, EphemeralSocialNote, SOCIAL_COMMITMENT_SCHEMA_VERSION,
    SOCIAL_EXCHANGE_EVENT_SCHEMA_VERSION, SOCIAL_EXCHANGE_EVENTS_FILENAME,
    SOCIAL_EXCHANGE_PACKET_SCHEMA_VERSION, SOCIAL_EXCHANGE_PROPOSAL_SCHEMA_VERSION,
    SOCIAL_EXCHANGE_STANCE_SCHEMA_VERSION, SocialCommitment, SocialCommitmentKind,
    SocialCommitmentMutation, SocialCommitmentStatus, SocialDueWindow, SocialExchangeActKind,
    SocialExchangeClosure, SocialExchangeEventPlan, SocialExchangeEventRecord,
    SocialExchangeMemory, SocialExchangeMutation, SocialExchangePacket, SocialExchangePolicy,
    SocialExchangeProposal, SocialIntensity, UNRESOLVED_SOCIAL_ASK_SCHEMA_VERSION,
    UnresolvedAskMutation, UnresolvedSocialAsk, append_social_exchange_event_plan,
    audit_social_exchange_contract, load_social_exchange_state, prepare_social_exchange_event_plan,
    rebuild_social_exchange,
};
pub use start::{
    StartWorldOptions, StartedWorld, render_started_world_report, start_world,
    world_seed_from_compact_text,
};
pub use store::{
    ACTIVE_WORLD_BINDING_SCHEMA_VERSION, ACTIVE_WORLD_FILENAME, ActiveWorldBinding,
    InitWorldOptions, InitializedWorld, SINGULARI_WORLD_HOME_ENV, StorePaths,
    WORLD_COMMIT_LOCK_SCHEMA_VERSION, WorldCommitLockClearReport, WorldCommitLockInfo,
    WorldCommitLockStatus, clear_world_commit_lock, init_world, latest_snapshot_path,
    load_active_world, load_latest_snapshot, load_world_record, resolve_store_paths,
    resolve_world_id, save_active_world, world_commit_lock_status,
};
pub use transfer::{
    EXPORT_MANIFEST_SCHEMA_VERSION, ExportManifest, ExportWorldOptions, ExportWorldReport,
    ImportWorldOptions, ImportWorldReport, export_world, import_world,
};
pub use turn::{AdvanceTurnOptions, AdvancedTurn, advance_turn, render_advanced_turn_report};
pub use turn_commit::{
    TURN_COMMIT_ENVELOPE_SCHEMA_VERSION, TURN_COMMIT_JOURNAL_RECOVERY_ACTION_SCHEMA_VERSION,
    TURN_COMMIT_JOURNAL_RECOVERY_SCHEMA_VERSION, TURN_COMMITS_FILENAME, TurnCommitEnvelope,
    TurnCommitJournalRecoveryAction, TurnCommitJournalRecoveryReport, TurnCommitStatus,
    TurnMaterializationRepairReport, append_turn_commit_envelope, recover_turn_commit_journal,
    repair_turn_materializations,
};
pub use turn_context::{
    TURN_CONTEXT_PACKET_SCHEMA_VERSION, TurnContextAssemblyPolicy, TurnContextPacket,
    assemble_turn_context_packet,
};
pub use turn_retrieval_controller::{
    TURN_RETRIEVAL_CONSTRAINT_SCHEMA_VERSION, TURN_RETRIEVAL_CONTROLLER_FILENAME,
    TURN_RETRIEVAL_CONTROLLER_SCHEMA_VERSION, TURN_RETRIEVAL_CUE_SCHEMA_VERSION,
    TURN_RETRIEVAL_EVENT_SCHEMA_VERSION, TURN_RETRIEVAL_EVENTS_FILENAME,
    TURN_RETRIEVAL_GOAL_SCHEMA_VERSION, TURN_RETRIEVAL_ROLE_STANCE_SCHEMA_VERSION,
    TurnRetrievalCompileInput, TurnRetrievalConstraint, TurnRetrievalControllerPacket,
    TurnRetrievalCue, TurnRetrievalCueReason, TurnRetrievalEventKind, TurnRetrievalEventRecord,
    TurnRetrievalGoal, TurnRetrievalGoalSource, TurnRetrievalPolicy, TurnRetrievalPriority,
    TurnRetrievalRoleStance, TurnRetrievalTargetKind, TurnRetrievalVisibility,
    compile_turn_retrieval_controller, load_turn_retrieval_controller_state,
};
pub use validate::{ValidationReport, ValidationStatus, validate_world};
pub use visual_asset_graph::{
    VISUAL_ASSET_GRAPH_FILENAME, VISUAL_ASSET_GRAPH_PACKET_SCHEMA_VERSION,
    VISUAL_ASSET_NODE_SCHEMA_VERSION, VisualAssetBoundary, VisualAssetGraphPacket,
    VisualAssetGraphPolicy, VisualAssetJobNode, VisualAssetNode, compile_visual_asset_graph_packet,
    load_visual_asset_graph_state, rebuild_visual_asset_graph,
};
pub use visual_assets::{
    BuildWorldVisualAssetsOptions, CHARACTER_SHEETS_DIR, ClaimVisualJobOptions,
    CompiledVisualPrompt, CompleteVisualJobOptions, HostImageGenerationCall, IMAGE_GENERATION_TOOL,
    ImageGenerationJob, LOCATION_SHEETS_DIR, MENU_BACKGROUND_FILENAME,
    ReleaseVisualJobClaimOptions, VISUAL_ASSETS_FILENAME, VISUAL_CANON_AUDIT_SCHEMA_VERSION,
    VISUAL_CANON_POLICY_SCHEMA_VERSION, VISUAL_JOB_CLAIM_RELEASE_SCHEMA_VERSION,
    VISUAL_JOB_CLAIM_SCHEMA_VERSION, VISUAL_JOB_COMPLETION_SCHEMA_VERSION, VN_ASSETS_DIR,
    VisualArtifactKind, VisualBudgetPolicy, VisualCanonAudit, VisualCanonAuditStatus,
    VisualCanonPolicy, VisualEntityAsset, VisualJobClaim, VisualJobClaimOutcome,
    VisualJobClaimRelease, VisualJobCompletion, WORLD_VISUAL_ASSETS_SCHEMA_VERSION,
    WorldVisualAsset, WorldVisualAssets, WorldVisualStyleProfile, apply_visual_canon_policy,
    build_world_visual_assets, claim_visual_job, compile_turn_visual_prompt, complete_visual_job,
    load_visual_job_claim, release_visual_job_claim, validate_visual_canon_policy_for_job,
    visual_canon_policy_prompt, visual_generation_job,
};
pub use vn::{
    BuildVnPacketOptions, VN_PACKET_SCHEMA_VERSION, VnAdjudication, VnChoice, VnHiddenFilter,
    VnPacket, VnScene, VnSceneImage, build_vn_packet,
};
pub use vn_server::{VnChooseRequest, VnServeOptions, serve_vn};
pub use voice_anchor::{
    ApplyCharacterAnchorOptions, CharacterAnchorReport, apply_character_anchor,
};
pub use world_court::{
    WORLD_COURT_REPAIR_ACTION_SCHEMA_VERSION, WORLD_COURT_VERDICT_SCHEMA_VERSION,
    WORLD_COURT_VIOLATION_SCHEMA_VERSION, WorldCourtInput, WorldCourtLayer, WorldCourtRepairAction,
    WorldCourtVerdict, WorldCourtVerdictStatus, WorldCourtViolation, WorldCourtViolationSeverity,
    adjudicate_world_changes, enforce_world_court_acceptance, render_world_court_verdict,
};
pub use world_db::{
    CanonEventRow, ChapterSummaryRecord, CharacterMemoryRow, EntityRecordRow, WORLD_DB_FILENAME,
    WORLD_DB_SCHEMA_VERSION, WorldDbRepairReport, WorldDbStats, WorldDbValidation, WorldFactRow,
    WorldSearchHit, force_chapter_summary, latest_chapter_summaries, recent_canon_events,
    recent_character_memories, recent_entity_updates, recent_relationship_updates, repair_world_db,
    search_world_db, sync_world_db_materialized_projections, validate_world_db,
    visible_entity_records, visible_world_facts, world_db_path, world_db_stats,
};
pub use world_docs::{WORLD_DOCS_DIR, refresh_world_docs, world_docs_dir};
pub use world_lore::{
    WORLD_LORE_ENTRY_SCHEMA_VERSION, WORLD_LORE_FILENAME, WORLD_LORE_PACKET_SCHEMA_VERSION,
    WORLD_LORE_UPDATE_SCHEMA_VERSION, WORLD_LORE_UPDATES_FILENAME, WorldLoreDomain, WorldLoreEntry,
    WorldLorePacket, WorldLorePolicy, WorldLoreUpdatePlan, WorldLoreUpdateRecord,
    append_world_lore_update_plan, build_world_lore_from_updates,
    compile_world_lore_from_projection, compile_world_lore_packet, load_world_lore_state,
    load_world_lore_update_records, prepare_world_lore_update_plan, rebuild_world_lore,
};
pub use world_process_clock::{
    WORLD_PROCESS_CLOCK_PACKET_SCHEMA_VERSION, WORLD_PROCESS_EVENT_SCHEMA_VERSION,
    WORLD_PROCESS_EVENTS_FILENAME, WORLD_PROCESS_SCHEMA_VERSION, WORLD_PROCESSES_FILENAME,
    WorldProcess, WorldProcessClockPacket, WorldProcessClockPolicy, WorldProcessEventPlan,
    WorldProcessEventRecord, WorldProcessTempo, WorldProcessTickPolicy, WorldProcessTickTrigger,
    WorldProcessVisibility, append_world_process_event_plan, compile_world_process_clock_packet,
    load_world_process_clock_state, merge_consequence_world_processes,
    prepare_world_process_event_plan, rebuild_world_process_clock,
};
