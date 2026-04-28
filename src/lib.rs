pub mod adjudication;
pub mod agent_bridge;
pub mod backend_selection;
pub mod chat;
pub mod codex_view;
pub mod entity_update;
pub mod extra_memory;
pub mod models;
pub mod render;
pub mod resume;
pub mod revival;
pub mod sqlite;
pub mod start;
pub mod store;
pub mod transfer;
pub mod turn;
pub mod validate;
pub mod visual_assets;
pub mod vn;
pub mod vn_server;
pub mod voice_anchor;
pub mod world_db;
pub mod world_docs;

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
pub use chat::{
    CHAT_ROUTE_SCHEMA_VERSION, ChatRoute, ChatRouteOptions, render_chat_route, route_chat_input,
};
pub use codex_view::{
    BuildCodexViewOptions, CodexViewSection, build_codex_view, render_codex_view_markdown,
    render_codex_view_section_markdown,
};
pub use entity_update::{EntityUpdateInput, apply_structured_entity_updates};
pub use extra_memory::{
    EXTRA_TRACE_SCHEMA_VERSION, EXTRA_TRACES_FILENAME, ExtraMemoryPacket, ExtraMemoryPolicy,
    ExtraMemoryRetrievalBudget, ExtraTrace, LocalFaceEntry, REMEMBERED_EXTRA_SCHEMA_VERSION,
    REMEMBERED_EXTRAS_FILENAME, REMEMBERED_EXTRAS_SCHEMA_VERSION, RememberedExtra,
    RememberedExtrasStore, apply_extra_memory_projection, commit_extra_memory_projection,
    compile_extra_memory_projection, local_faces_for_codex_view, retrieve_extra_memory_packet,
};
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
pub use render::{RenderPacketLoadOptions, load_render_packet, render_packet_markdown};
pub use resume::{
    BuildResumePackOptions, HiddenStateSummary, RESUME_PACK_SCHEMA_VERSION, ResumePack,
    build_resume_pack, render_resume_pack_markdown,
};
pub use revival::{
    AGENT_REVIVAL_PACKET_SCHEMA_VERSION, AgentRevivalCompileOptions, build_agent_revival_packet,
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
pub use validate::{ValidationReport, ValidationStatus, validate_world};
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
    search_world_db, validate_world_db, visible_entity_records, visible_world_facts, world_db_path,
    world_db_stats,
};
pub use world_docs::{WORLD_DOCS_DIR, refresh_world_docs, world_docs_dir};
