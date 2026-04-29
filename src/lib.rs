pub mod adjudication;
pub mod agent_bridge;
pub mod backend_selection;
pub mod body_resource;
pub mod character_text_design;
pub mod chat;
pub mod codex_view;
pub mod entity_update;
pub mod extra_memory;
pub mod host_supervisor;
pub mod job_ledger;
pub mod location_graph;
pub mod memory_revival_policy;
pub mod models;
pub mod plot_thread;
pub mod projection_health;
pub mod relationship_graph;
pub mod render;
pub mod response_context;
pub mod resume;
pub mod revival;
pub mod scene_pressure;
pub mod sqlite;
pub mod start;
pub mod store;
pub mod transfer;
pub mod turn;
pub mod turn_commit;
pub mod turn_context;
pub mod validate;
pub mod visual_asset_graph;
pub mod visual_assets;
pub mod vn;
pub mod vn_server;
pub mod voice_anchor;
pub mod world_db;
pub mod world_docs;
pub mod world_lore;

pub use adjudication::{AdjudicationInput, adjudicate_turn};
pub use agent_bridge::{
    AGENT_COMMIT_RECORD_SCHEMA_VERSION, AGENT_PENDING_TURN_SCHEMA_VERSION,
    AGENT_TURN_RESPONSE_SCHEMA_VERSION, AgentCommitTurnOptions, AgentExtraContact,
    AgentHiddenSecret, AgentHiddenTimer, AgentOutputContract, AgentPrivateAdjudicationContext,
    AgentResponseAdjudication, AgentResponseCanonEvent, AgentSubmitTurnOptions, AgentTurnResponse,
    AgentVisibleContext, AgentVoiceAnchor, CommittedAgentTurn, PendingAgentChoice,
    PendingAgentTurn, commit_agent_turn, enqueue_agent_turn, load_pending_agent_turn,
};
pub use backend_selection::{
    WORLD_BACKEND_SELECTION_FILENAME, WORLD_BACKEND_SELECTION_SCHEMA_VERSION,
    WorldBackendSelection, WorldTextBackend, WorldVisualBackend, backend_selection_path,
    load_world_backend_selection, save_world_backend_selection,
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
pub use entity_update::{EntityUpdateInput, apply_structured_entity_updates};
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
    read_world_jobs,
};
pub use location_graph::{
    LOCATION_EVENT_SCHEMA_VERSION, LOCATION_EVENTS_FILENAME, LOCATION_GRAPH_FILENAME,
    LOCATION_GRAPH_PACKET_SCHEMA_VERSION, LOCATION_NODE_SCHEMA_VERSION, LocationEvent,
    LocationEventKind, LocationEventPlan, LocationEventRecord, LocationGraphPacket,
    LocationGraphPolicy, LocationKnowledgeState, LocationNode, append_location_event_plan,
    compile_location_graph_packet, load_location_graph_state, prepare_location_event_plan,
    rebuild_location_graph,
};
pub use memory_revival_policy::{MEMORY_REVIVAL_POLICY_SCHEMA_VERSION, MemoryRevivalPolicy};
pub use models::{
    ADJUDICATION_SCHEMA_VERSION, ANCHOR_CHARACTER_ID, ANCHOR_CHARACTER_INVARIANT, AdjudicationGate,
    AdjudicationReport, AnchorCharacter, CANON_EVENT_SCHEMA_VERSION, CODEX_VIEW_SCHEMA_VERSION,
    CanonEvent, CharacterBody, CharacterRecord, CharacterVoiceAnchor, CodexAnalysisEntry,
    CodexEntityEntry, CodexFactEntry, CodexHiddenFilter, CodexRecommendation, CodexTimelineEntry,
    CodexView, CodexVoiceAnchorEntry, CurrentEvent, DEFAULT_CHOICE_COUNT, DashboardSummary,
    ENTITY_UPDATE_SCHEMA_VERSION, EntityName, EntityRecords, EntityUpdateRecord,
    FREEFORM_CHOICE_SLOT, FREEFORM_CHOICE_TAG, GUIDE_CHOICE_REDACTED_INTENT, GUIDE_CHOICE_SLOT,
    HiddenState, HiddenStateSecret, HiddenStateTimer, NARRATIVE_SCENE_SCHEMA_VERSION,
    NarrativeScene, PlayerKnowledge, ProtagonistState, RENDER_PACKET_SCHEMA_VERSION,
    RelationshipUpdateRecord, RenderPacket, SINGULARI_WORLD_SCHEMA_VERSION, ScanTarget,
    StructuredEntityUpdates, TURN_LOG_ENTRY_SCHEMA_VERSION, TURN_SNAPSHOT_SCHEMA_VERSION,
    TurnChoice, TurnInputKind, TurnLogEntry, TurnSnapshot, VisibleState, WorldPremise, WorldRecord,
    WorldSeed, default_freeform_choice, default_turn_choices, normalize_turn_choices,
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
pub use projection_health::{
    PROJECTION_HEALTH_SCHEMA_VERSION, ProjectionComponentHealth, ProjectionHealthReport,
    ProjectionHealthStatus, build_projection_health_report, render_projection_health_report,
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
pub use scene_pressure::{
    ACTIVE_SCENE_PRESSURES_FILENAME, SCENE_PRESSURE_AUDIT_FILENAME,
    SCENE_PRESSURE_AUDIT_SCHEMA_VERSION, SCENE_PRESSURE_EVENT_SCHEMA_VERSION,
    SCENE_PRESSURE_EVENTS_FILENAME, SCENE_PRESSURE_PACKET_SCHEMA_VERSION,
    SCENE_PRESSURE_SCHEMA_VERSION, ScenePressure, ScenePressureAuditRecord, ScenePressureChange,
    ScenePressureEvent, ScenePressureEventPlan, ScenePressureEventRecord, ScenePressureKind,
    ScenePressurePacket, ScenePressurePolicy, ScenePressureProseEffect, ScenePressureUrgency,
    ScenePressureVisibility, append_scene_pressure_audit, append_scene_pressure_event_plan,
    compile_scene_pressure_packet, load_active_scene_pressures, prepare_scene_pressure_event_plan,
    rebuild_active_scene_pressures,
};
pub use start::{
    StartWorldOptions, StartedWorld, render_started_world_report, start_world,
    world_seed_from_compact_text,
};
pub use store::{
    ACTIVE_WORLD_BINDING_SCHEMA_VERSION, ACTIVE_WORLD_FILENAME, ActiveWorldBinding,
    InitWorldOptions, InitializedWorld, SINGULARI_WORLD_HOME_ENV, StorePaths, init_world,
    latest_snapshot_path, load_active_world, load_latest_snapshot, load_world_record,
    resolve_store_paths, resolve_world_id, save_active_world,
};
pub use transfer::{
    EXPORT_MANIFEST_SCHEMA_VERSION, ExportManifest, ExportWorldOptions, ExportWorldReport,
    ImportWorldOptions, ImportWorldReport, export_world, import_world,
};
pub use turn::{AdvanceTurnOptions, AdvancedTurn, advance_turn, render_advanced_turn_report};
pub use turn_commit::{
    TURN_COMMIT_ENVELOPE_SCHEMA_VERSION, TURN_COMMITS_FILENAME, TurnCommitEnvelope,
    TurnCommitStatus, TurnMaterializationRepairReport, append_turn_commit_envelope,
    repair_turn_materializations,
};
pub use turn_context::{
    TURN_CONTEXT_PACKET_SCHEMA_VERSION, TurnContextAssemblyPolicy, TurnContextPacket,
    assemble_turn_context_packet,
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
    ReleaseVisualJobClaimOptions, VISUAL_ASSETS_FILENAME, VISUAL_JOB_CLAIM_RELEASE_SCHEMA_VERSION,
    VISUAL_JOB_CLAIM_SCHEMA_VERSION, VISUAL_JOB_COMPLETION_SCHEMA_VERSION, VN_ASSETS_DIR,
    VisualArtifactKind, VisualBudgetPolicy, VisualEntityAsset, VisualJobClaim,
    VisualJobClaimOutcome, VisualJobClaimRelease, VisualJobCompletion,
    WORLD_VISUAL_ASSETS_SCHEMA_VERSION, WorldVisualAsset, WorldVisualAssets,
    WorldVisualStyleProfile, build_world_visual_assets, claim_visual_job,
    compile_turn_visual_prompt, complete_visual_job, load_visual_job_claim,
    release_visual_job_claim, visual_generation_job,
};
pub use vn::{
    BuildVnPacketOptions, VN_PACKET_SCHEMA_VERSION, VnAdjudication, VnChoice, VnHiddenFilter,
    VnPacket, VnScene, VnSceneImage, build_vn_packet,
};
pub use vn_server::{VnChooseRequest, VnServeOptions, serve_vn};
pub use voice_anchor::{
    ApplyCharacterAnchorOptions, CharacterAnchorReport, apply_character_anchor,
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
