use crate::actor_agency::{
    ActorAgencyPacket, AgentActorGoalUpdate, AgentActorMoveUpdate, append_actor_agency_event_plan,
    load_actor_agency_state, merge_consequence_actor_agency, merge_social_exchange_actor_agency,
    prepare_actor_agency_event_plan, rebuild_actor_agency_packet,
};
use crate::autobiographical_index::{
    AutobiographicalIndexInput, AutobiographicalIndexPacket, rebuild_autobiographical_index,
};
use crate::belief_graph::{
    BeliefGraphPacket, append_belief_event_plan, load_belief_graph_state,
    prepare_belief_event_plan, rebuild_belief_graph,
};
use crate::body_resource::{
    BodyResourceEvent, BodyResourcePacket, append_body_resource_event_plan,
    prepare_body_resource_event_plan, rebuild_body_resource_state,
};
use crate::change_ledger::{
    ChangeEventPlanInput, ChangeLedgerPacket, append_change_event_plan, load_change_ledger_state,
    prepare_change_event_plan, rebuild_change_ledger,
};
use crate::character_text_design::{
    CharacterTextDesignPacket, append_character_text_design_event_plan,
    compile_character_text_design_with_projection, load_character_text_design_state,
    prepare_character_text_design_event_plan, rebuild_character_text_design,
};
use crate::consequence_spine::{
    ConsequenceProposal, ConsequenceSpinePacket, append_consequence_event_plan,
    audit_consequence_contract, load_consequence_spine_state, prepare_consequence_event_plan,
    rebuild_consequence_spine,
};
use crate::context_capsule::{
    ContextCapsuleBuildInput, ContextCapsuleIndex, ContextCapsuleSelection,
    ContextCapsuleSelectionInput, rebuild_context_capsule_registry, select_context_capsules,
};
use crate::encounter_surface::{
    EncounterProposal, EncounterSurfacePacket, append_encounter_surface_event_plan,
    compile_encounter_surface_packet, load_encounter_surface_state,
    prepare_encounter_surface_event_plan, rebuild_encounter_surface,
};
use crate::extra_memory::{
    ExtraMemoryPacket, commit_extra_memory_projection_terminal, compile_extra_memory_projection,
    retrieve_extra_memory_packet,
};
use crate::location_graph::{
    LocationEvent, LocationGraphPacket, append_location_event_plan, compile_location_graph_packet,
    load_location_graph_state, prepare_location_event_plan, rebuild_location_graph,
};
use crate::models::{
    AdjudicationGate, CharacterVoiceAnchor, FREEFORM_CHOICE_SLOT, GUIDE_CHOICE_SLOT, HiddenState,
    NARRATIVE_SCENE_SCHEMA_VERSION, NarrativeScene, TurnChoice, TurnSnapshot,
    default_freeform_choice, default_turn_choices, is_guide_choice_tag, normalize_turn_choices,
};
use crate::narrative_style_state::{
    NarrativeStyleState, append_narrative_style_event_plan, compile_narrative_style_state,
    load_narrative_style_state, prepare_narrative_style_event_plan, rebuild_narrative_style_state,
};
use crate::pattern_debt::{
    PatternDebtPacket, append_pattern_debt_event_plan, load_pattern_debt_state,
    prepare_pattern_debt_event_plan, rebuild_pattern_debt,
};
use crate::player_intent::{
    PlayerIntentTracePacket, append_player_intent_event_plan, load_player_intent_trace_state,
    prepare_player_intent_event_plan, rebuild_player_intent_trace,
};
use crate::plot_thread::{
    PlotThreadEvent, PlotThreadPacket, append_plot_thread_audit, append_plot_thread_event_plan,
    compile_plot_thread_packet, load_plot_threads, prepare_plot_thread_event_plan,
    rebuild_plot_threads,
};
use crate::projection_registry::load_body_resource_prompt_packet;
use crate::prompt_context::{CompilePromptContextPacketOptions, compile_prompt_context_packet};
use crate::relationship_graph::{
    RelationshipGraphPacket, append_relationship_graph_event_plan,
    compile_relationship_graph_from_projection, load_relationship_graph_state,
    prepare_relationship_graph_event_plan, rebuild_relationship_graph,
};
use crate::resolution::{
    ResolutionCritique, ResolutionProposal, audit_resolution_choices, audit_resolution_proposal,
};
use crate::response_context::{
    AgentCharacterTextDesignUpdate, AgentContextEventInput, AgentEntityUpdate,
    AgentHiddenStateDelta, AgentRelationshipUpdate, AgentWorldLoreUpdate,
    append_agent_context_event_plan, load_agent_context_projection,
    prepare_agent_context_event_plan, rebuild_agent_context_projection,
};
use crate::scene_director::{
    SceneDirectorCompileInput, SceneDirectorCritique, SceneDirectorPacket, SceneDirectorProposal,
    append_scene_director_event_plan, audit_scene_director_proposal,
    compile_scene_director_packet_from_input, load_scene_director_state,
    merge_scene_director_history, prepare_scene_director_event_plan, rebuild_scene_director,
};
use crate::scene_pressure::{
    ScenePressureEvent, ScenePressurePacket, append_scene_pressure_audit,
    append_scene_pressure_event_plan, compile_scene_pressure_packet, load_active_scene_pressures,
    merge_consequence_scene_pressures, merge_social_exchange_scene_pressures,
    prepare_scene_pressure_event_plan, rebuild_active_scene_pressures,
};
use crate::social_exchange::{
    SocialExchangePacket, SocialExchangeProposal, append_social_exchange_event_plan,
    audit_social_exchange_contract, load_social_exchange_state, prepare_social_exchange_event_plan,
    rebuild_social_exchange,
};
use crate::store::{
    WorldFilePaths, append_jsonl, read_json, resolve_store_paths, world_file_paths, write_json,
};
use crate::turn::{AdvanceTurnOptions, advance_turn};
use crate::turn_commit::{TurnCommitEnvelope, append_turn_commit_envelope};
use crate::turn_retrieval_controller::{
    TurnRetrievalCompileInput, TurnRetrievalControllerPacket, compile_turn_retrieval_controller,
};
use crate::vn::{BuildVnPacketOptions, VnPacket, build_vn_packet};
use crate::world_db::latest_chapter_summaries;
use crate::world_lore::{
    WorldLorePacket, append_world_lore_update_plan, compile_world_lore_from_projection,
    load_world_lore_state, prepare_world_lore_update_plan, rebuild_world_lore,
};
use crate::world_process_clock::{
    WorldProcessClockPacket, append_world_process_event_plan, load_world_process_clock_state,
    merge_consequence_world_processes, prepare_world_process_event_plan,
    rebuild_world_process_clock,
};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const AGENT_PENDING_TURN_SCHEMA_VERSION: &str = "singulari.agent_pending_turn.v1";
pub const AGENT_TURN_RESPONSE_SCHEMA_VERSION: &str = "singulari.agent_turn_response.v1";
pub const AGENT_COMMIT_RECORD_SCHEMA_VERSION: &str = "singulari.agent_commit_record.v1";
pub const POST_COMMIT_MATERIALIZATION_EVENT_SCHEMA_VERSION: &str =
    "singulari.post_commit_materialization_event.v1";

const AGENT_BRIDGE_DIR: &str = "agent_bridge";
const PENDING_AGENT_TURN_FILENAME: &str = "pending_turn.json";
const COMMITTED_AGENT_TURNS_DIR: &str = "committed_turns";
const AGENT_COMMIT_RECORD_FILENAME: &str = "commit_record.json";
const POST_COMMIT_MATERIALIZATION_EVENTS_FILENAME: &str =
    "post_commit_materialization_events.jsonl";

#[derive(Debug, Clone)]
pub struct AgentSubmitTurnOptions {
    pub store_root: Option<PathBuf>,
    pub world_id: String,
    pub input: String,
    pub narrative_level: Option<u8>,
}

#[derive(Debug, Clone)]
pub struct AgentCommitTurnOptions {
    pub store_root: Option<PathBuf>,
    pub world_id: String,
    pub response: AgentTurnResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingAgentTurn {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub status: String,
    pub player_input: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_choice: Option<PendingAgentChoice>,
    pub visible_context: AgentVisibleContext,
    pub private_adjudication_context: AgentPrivateAdjudicationContext,
    pub output_contract: AgentOutputContract,
    pub pending_ref: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingAgentChoice {
    pub slot: u8,
    pub tag: String,
    pub visible_intent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentVisibleContext {
    pub location: String,
    #[serde(default)]
    pub recent_scene: Vec<String>,
    #[serde(default)]
    pub known_facts: Vec<String>,
    #[serde(default)]
    pub voice_anchors: Vec<AgentVoiceAnchor>,
    #[serde(default)]
    pub extra_memory: ExtraMemoryPacket,
    #[serde(default)]
    pub active_scene_pressure: ScenePressurePacket,
    #[serde(default)]
    pub active_plot_threads: PlotThreadPacket,
    #[serde(default)]
    pub active_body_resource_state: BodyResourcePacket,
    #[serde(default)]
    pub active_location_graph: LocationGraphPacket,
    #[serde(default)]
    pub active_character_text_design: CharacterTextDesignPacket,
    #[serde(default)]
    pub active_world_lore: WorldLorePacket,
    #[serde(default)]
    pub active_relationship_graph: RelationshipGraphPacket,
    #[serde(default)]
    pub active_actor_agency: ActorAgencyPacket,
    #[serde(default)]
    pub active_change_ledger: ChangeLedgerPacket,
    #[serde(default)]
    pub active_pattern_debt: PatternDebtPacket,
    #[serde(default)]
    pub active_belief_graph: BeliefGraphPacket,
    #[serde(default)]
    pub active_world_process_clock: WorldProcessClockPacket,
    #[serde(default)]
    pub active_player_intent_trace: PlayerIntentTracePacket,
    #[serde(default)]
    pub active_narrative_style_state: NarrativeStyleState,
    #[serde(default)]
    pub active_scene_director: SceneDirectorPacket,
    #[serde(default)]
    pub active_consequence_spine: ConsequenceSpinePacket,
    #[serde(default)]
    pub active_social_exchange: SocialExchangePacket,
    #[serde(default)]
    pub active_encounter_surface: EncounterSurfacePacket,
    #[serde(default)]
    pub active_turn_retrieval_controller: TurnRetrievalControllerPacket,
    #[serde(default)]
    pub selected_context_capsules: ContextCapsuleSelection,
    #[serde(default)]
    pub active_autobiographical_index: AutobiographicalIndexPacket,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentVoiceAnchor {
    pub character_id: String,
    pub name: String,
    pub anchor: CharacterVoiceAnchor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPrivateAdjudicationContext {
    #[serde(default)]
    pub hidden_timers: Vec<AgentHiddenTimer>,
    #[serde(default)]
    pub unrevealed_constraints: Vec<AgentHiddenSecret>,
    #[serde(default)]
    pub plausibility_gates: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHiddenTimer {
    pub timer_id: String,
    pub kind: String,
    pub remaining_turns: u32,
    pub effect: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHiddenSecret {
    pub secret_id: String,
    pub status: String,
    pub truth: String,
    #[serde(default)]
    pub reveal_conditions: Vec<String>,
    #[serde(default)]
    pub forbidden_leaks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentOutputContract {
    pub language: String,
    pub must_return_json: bool,
    pub hidden_truth_must_not_appear_in_visible_text: bool,
    #[serde(default = "default_narrative_level")]
    pub narrative_level: u8,
    #[serde(default = "default_narrative_budget")]
    pub narrative_budget: AgentNarrativeBudget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentNarrativeBudget {
    pub level_label: String,
    pub ordinary_turn_blocks: u8,
    pub standard_choice_turn_blocks: u8,
    pub major_turn_blocks: u8,
    pub opening_or_climax_blocks: u8,
    pub target_chars: u32,
    pub major_target_chars: u32,
    pub ordinary_turn: String,
    pub standard_choice_turn: String,
    pub major_turn: String,
    pub opening_or_climax: String,
    pub character_budget: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTurnResponse {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution_proposal: Option<ResolutionProposal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_director_proposal: Option<SceneDirectorProposal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consequence_proposal: Option<ConsequenceProposal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub social_exchange_proposal: Option<SocialExchangeProposal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encounter_proposal: Option<EncounterProposal>,
    pub visible_scene: NarrativeScene,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<AgentResponseAdjudication>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canon_event: Option<AgentResponseCanonEvent>,
    #[serde(default)]
    pub entity_updates: Vec<AgentEntityUpdate>,
    #[serde(default)]
    pub relationship_updates: Vec<AgentRelationshipUpdate>,
    #[serde(default)]
    pub plot_thread_events: Vec<PlotThreadEvent>,
    #[serde(default)]
    pub scene_pressure_events: Vec<ScenePressureEvent>,
    #[serde(default)]
    pub world_lore_updates: Vec<AgentWorldLoreUpdate>,
    #[serde(default)]
    pub character_text_design_updates: Vec<AgentCharacterTextDesignUpdate>,
    #[serde(default)]
    pub body_resource_events: Vec<BodyResourceEvent>,
    #[serde(default)]
    pub location_events: Vec<LocationEvent>,
    #[serde(default)]
    pub extra_contacts: Vec<AgentExtraContact>,
    #[serde(default)]
    pub hidden_state_delta: Vec<AgentHiddenStateDelta>,
    #[serde(default)]
    pub needs_context: Vec<AgentContextRepairRequest>,
    #[serde(default)]
    pub next_choices: Vec<TurnChoice>,
    #[serde(default)]
    pub actor_goal_events: Vec<AgentActorGoalUpdate>,
    #[serde(default)]
    pub actor_move_events: Vec<AgentActorMoveUpdate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentContextRepairRequest {
    pub request_id: String,
    #[serde(default)]
    pub capsule_kinds: Vec<String>,
    pub query: String,
    pub reason: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentExtraContact {
    pub surface_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_role: Option<String>,
    pub contact_summary: String,
    #[serde(default)]
    pub pressure_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promotion_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disposition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_design: Option<CharacterVoiceAnchor>,
    #[serde(default)]
    pub open_hooks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponseAdjudication {
    pub outcome: String,
    pub summary: String,
    #[serde(default)]
    pub gates: Vec<AdjudicationGate>,
    #[serde(default)]
    pub visible_constraints: Vec<String>,
    #[serde(default)]
    pub consequences: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponseCanonEvent {
    pub visibility: String,
    pub kind: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommittedAgentTurn {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub render_packet_path: String,
    pub response_path: String,
    pub commit_record_path: String,
    pub committed_at: String,
    pub packet: VnPacket,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostCommitMaterializationEvent {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub component: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub recorded_at: String,
}

/// Queue a player input for local-agent narrative authorship.
///
/// # Errors
///
/// Returns an error when the world cannot be loaded, a previous pending turn is
/// still open, or the input is empty.
pub fn enqueue_agent_turn(options: &AgentSubmitTurnOptions) -> Result<PendingAgentTurn> {
    let player_input = options.input.trim();
    if player_input.is_empty() {
        bail!("agent bridge input must not be empty");
    }
    let store_paths = resolve_store_paths(options.store_root.as_deref())?;
    let files = world_file_paths(&store_paths, options.world_id.as_str());
    let pending_path = pending_agent_turn_path(&files);
    if pending_path.exists() {
        let pending: PendingAgentTurn = read_json(&pending_path)?;
        if pending.status == "pending" {
            bail!(
                "agent turn already pending: world_id={}, turn_id={}, pending_ref={}",
                pending.world_id,
                pending.turn_id,
                pending.pending_ref
            );
        }
    }

    let snapshot: TurnSnapshot = read_json(&files.latest_snapshot)?;
    let hidden_state: HiddenState = read_json(&files.hidden_state)?;
    let entities: crate::models::EntityRecords = read_json(&files.entities)?;
    let current_packet = build_vn_packet(&BuildVnPacketOptions {
        store_root: options.store_root.clone(),
        world_id: options.world_id.clone(),
        turn_id: None,
        scene_image_url: None,
    })?;
    let turn_id = next_turn_id(snapshot.turn_id.as_str())?;
    let pending_ref = pending_path.display().to_string();
    let pending_choice = selected_choice(player_input, &snapshot);
    let private_adjudication_context = private_context(&hidden_state);
    let output_contract = AgentOutputContract {
        language: "ko".to_owned(),
        must_return_json: true,
        hidden_truth_must_not_appear_in_visible_text: true,
        narrative_level: normalize_narrative_level(options.narrative_level),
        narrative_budget: narrative_budget_for_level(options.narrative_level),
    };
    let visible_context = visible_context(&VisibleContextInput {
        files: &files,
        snapshot: &snapshot,
        entities: &entities,
        current_packet: &current_packet,
        selected_choice: pending_choice.as_ref(),
        private_context: &private_adjudication_context,
        player_input,
        output_contract: &output_contract,
    })?;
    append_scene_pressure_audit(files.dir.as_path(), &visible_context.active_scene_pressure)?;
    append_plot_thread_audit(files.dir.as_path(), &visible_context.active_plot_threads)?;
    let pending = PendingAgentTurn {
        schema_version: AGENT_PENDING_TURN_SCHEMA_VERSION.to_owned(),
        world_id: options.world_id.clone(),
        turn_id,
        status: "pending".to_owned(),
        player_input: player_input.to_owned(),
        selected_choice: pending_choice,
        visible_context,
        private_adjudication_context,
        output_contract,
        pending_ref,
        created_at: Utc::now().to_rfc3339(),
    };
    ensure_parent_dir(&pending_path)?;
    write_json(&pending_path, &pending)?;
    Ok(pending)
}

#[must_use]
pub fn normalize_narrative_level(level: Option<u8>) -> u8 {
    level.unwrap_or(default_narrative_level()).clamp(1, 3)
}

#[must_use]
pub fn narrative_budget_for_level(level: Option<u8>) -> AgentNarrativeBudget {
    match normalize_narrative_level(level) {
        1 => AgentNarrativeBudget {
            level_label: "서사레벨 1: 표준 VN 밀도".to_owned(),
            ordinary_turn_blocks: 3,
            standard_choice_turn_blocks: 6,
            major_turn_blocks: 8,
            opening_or_climax_blocks: 10,
            target_chars: 1_400,
            major_target_chars: 2_200,
            ordinary_turn: "일상/이동/짧은 관찰은 2-3문단".to_owned(),
            standard_choice_turn: "기본 선택 턴은 4-6문단".to_owned(),
            major_turn: "전투 시작, 첫 인물 등장, 장소 전환, 비밀 단서 발견은 6-8문단".to_owned(),
            opening_or_climax: "챕터 오프닝/클라이맥스는 8-10문단".to_owned(),
            character_budget: "기본 턴 900-1400자, 큰 턴 1600-2200자".to_owned(),
        },
        2 => AgentNarrativeBudget {
            level_label: "서사레벨 2: 장면 확장 밀도".to_owned(),
            ordinary_turn_blocks: 5,
            standard_choice_turn_blocks: 9,
            major_turn_blocks: 12,
            opening_or_climax_blocks: 14,
            target_chars: 3_400,
            major_target_chars: 4_800,
            ordinary_turn: "일상/이동/짧은 관찰도 4-5문단".to_owned(),
            standard_choice_turn: "기본 선택 턴은 7-9문단".to_owned(),
            major_turn: "전투 시작, 첫 인물 등장, 장소 전환, 비밀 단서 발견은 10-12문단".to_owned(),
            opening_or_climax: "챕터 오프닝/클라이맥스는 12-14문단".to_owned(),
            character_budget: "기본 턴 2200-3400자, 큰 턴 3600-4800자".to_owned(),
        },
        _ => AgentNarrativeBudget {
            level_label: "서사레벨 3: 장편 연재 밀도".to_owned(),
            ordinary_turn_blocks: 9,
            standard_choice_turn_blocks: 14,
            major_turn_blocks: 20,
            opening_or_climax_blocks: 24,
            target_chars: 7_000,
            major_target_chars: 12_000,
            ordinary_turn: "일상/이동/짧은 관찰도 7-9문단".to_owned(),
            standard_choice_turn: "기본 선택 턴은 11-14문단".to_owned(),
            major_turn: "전투 시작, 첫 인물 등장, 장소 전환, 비밀 단서 발견은 16-20문단".to_owned(),
            opening_or_climax: "챕터 오프닝/클라이맥스는 20-24문단".to_owned(),
            character_budget: "기본 턴 4500-7000자, 큰 턴 8000-12000자".to_owned(),
        },
    }
}

const fn default_narrative_level() -> u8 {
    1
}

fn default_narrative_budget() -> AgentNarrativeBudget {
    narrative_budget_for_level(Some(default_narrative_level()))
}

/// Load the current pending agent turn.
///
/// # Errors
///
/// Returns an error when no pending turn exists or the pending file cannot be parsed.
pub fn load_pending_agent_turn(
    store_root: Option<&Path>,
    world_id: &str,
) -> Result<PendingAgentTurn> {
    let store_paths = resolve_store_paths(store_root)?;
    let files = world_file_paths(&store_paths, world_id);
    let mut pending: PendingAgentTurn = read_json(&pending_agent_turn_path(&files))?;
    let snapshot: TurnSnapshot = read_json(&files.latest_snapshot)?;
    pending.selected_choice = selected_choice(pending.player_input.as_str(), &snapshot);
    Ok(pending)
}

/// Commit an agent-authored scene and advance the world by the queued input.
///
/// # Errors
///
/// Returns an error when there is no matching pending turn, hidden truth leaks
/// into visible text, or turn persistence fails.
#[allow(clippy::too_many_lines)]
pub fn commit_agent_turn(options: &AgentCommitTurnOptions) -> Result<CommittedAgentTurn> {
    let store_paths = resolve_store_paths(options.store_root.as_deref())?;
    let files = world_file_paths(&store_paths, options.world_id.as_str());
    let pending_path = pending_agent_turn_path(&files);
    let pending =
        load_pending_agent_turn(options.store_root.as_deref(), options.world_id.as_str())?;
    let response = canonical_agent_turn_response(options.response.clone());
    validate_agent_response(&pending, &response)?;
    audit_agent_resolution_proposal(options.store_root.as_deref(), &pending, &response)?;
    audit_agent_scene_director_proposal(options.store_root.as_deref(), &pending, &response)?;
    let context_event_plan = prepare_agent_context_event_plan(
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        &AgentContextEventInput {
            entity_updates: &response.entity_updates,
            relationship_updates: &response.relationship_updates,
            world_lore_updates: &response.world_lore_updates,
            character_text_design_updates: &response.character_text_design_updates,
            hidden_state_delta: &response.hidden_state_delta,
        },
    )?;
    let relationship_graph_event_plan = prepare_relationship_graph_event_plan(
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        &response.relationship_updates,
    )?;
    let actor_agency_event_plan = prepare_actor_agency_event_plan(
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        &pending.visible_context.active_relationship_graph,
        &response,
    )?;
    let world_lore_update_plan = prepare_world_lore_update_plan(
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        &response.world_lore_updates,
    )?;
    let character_text_design_event_plan = prepare_character_text_design_event_plan(
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        &response.character_text_design_updates,
    )?;
    let plot_thread_event_plan = prepare_plot_thread_event_plan(
        &pending.visible_context.active_plot_threads,
        &response.plot_thread_events,
    )?;
    let scene_pressure_event_plan = prepare_scene_pressure_event_plan(
        &pending.visible_context.active_scene_pressure,
        &response.scene_pressure_events,
    )?;
    let prompt_context = compile_prompt_context_packet(&CompilePromptContextPacketOptions {
        store_root: options.store_root.as_deref(),
        pending: &pending,
        engine_session_kind: "webgpt_project_session",
    })?;
    let active_scene_director: SceneDirectorPacket =
        serde_json::from_value(prompt_context.visible_context.active_scene_director.clone())
            .context("prompt context active_scene_director is invalid")?;
    let scene_director_event_plan = prepare_scene_director_event_plan(
        &active_scene_director,
        response.scene_director_proposal.as_ref(),
    )?;
    audit_consequence_contract(
        &prompt_context,
        &pending.visible_context.active_consequence_spine,
        response.consequence_proposal.as_ref(),
        response.resolution_proposal.as_ref(),
    )
    .context("consequence proposal audit failed")?;
    audit_social_exchange_contract(
        &prompt_context,
        &pending.visible_context.active_social_exchange,
        &response,
    )
    .context("social exchange proposal audit failed")?;
    let body_resource_event_plan = prepare_body_resource_event_plan(
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        &response.body_resource_events,
    )?;
    let location_event_plan = prepare_location_event_plan(
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        &response.location_events,
    )?;
    let change_event_plan = prepare_change_event_plan(ChangeEventPlanInput {
        world_id: options.world_id.as_str(),
        turn_id: pending.turn_id.as_str(),
        relationship_graph_events: &relationship_graph_event_plan,
        world_lore_updates: &world_lore_update_plan,
        character_text_design_events: &character_text_design_event_plan,
        plot_thread_events: &plot_thread_event_plan,
        scene_pressure_events: &scene_pressure_event_plan,
    })?;
    let pattern_debt_event_plan = prepare_pattern_debt_event_plan(
        files.dir.as_path(),
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        &response,
    )?;
    let belief_event_plan = prepare_belief_event_plan(
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        &response,
    );
    let world_process_event_plan = prepare_world_process_event_plan(
        &pending.visible_context.active_world_process_clock,
        &scene_pressure_event_plan,
        response.resolution_proposal.as_ref(),
    );
    let consequence_event_plan = prepare_consequence_event_plan(
        &pending.visible_context.active_consequence_spine,
        response.consequence_proposal.as_ref(),
        response.resolution_proposal.as_ref(),
        response.social_exchange_proposal.as_ref(),
    )?;
    let social_exchange_event_plan = prepare_social_exchange_event_plan(
        &pending.visible_context.active_social_exchange,
        &response,
        &pending.visible_context.active_consequence_spine,
    )?;
    let encounter_surface_event_plan = prepare_encounter_surface_event_plan(
        &pending.visible_context.active_encounter_surface,
        &response,
    )?;
    let player_intent_event_plan = prepare_player_intent_event_plan(
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        pending.player_input.as_str(),
        pending.selected_choice.as_ref(),
        &response,
    );
    let narrative_style_event_plan = prepare_narrative_style_event_plan(
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        &response,
    );
    let extra_memory_projection = compile_extra_memory_projection(
        files.dir.as_path(),
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        pending.visible_context.location.as_str(),
        &response.extra_contacts,
    )?;
    let before_commit_snapshot: TurnSnapshot = read_json(&files.latest_snapshot)?;
    append_turn_commit_envelope(
        files.dir.as_path(),
        &TurnCommitEnvelope::prepared(
            &pending,
            before_commit_snapshot.turn_id.as_str(),
            Utc::now().to_rfc3339(),
        ),
    )?;

    let advanced = advance_turn(&AdvanceTurnOptions {
        store_root: options.store_root.clone(),
        world_id: options.world_id.clone(),
        input: pending.player_input.clone(),
    })?;
    if advanced.snapshot.turn_id != pending.turn_id {
        bail!(
            "agent bridge turn mismatch after advance: pending={}, advanced={}",
            pending.turn_id,
            advanced.snapshot.turn_id
        );
    }

    let mut render_packet = advanced.render_packet;
    apply_agent_response_to_render_packet(&mut render_packet, &response);
    write_json(&advanced.render_packet_path, &render_packet)?;

    persist_agent_next_choices(
        &files,
        &advanced.snapshot_path,
        &advanced.snapshot,
        &response.next_choices,
    )?;

    let committed_at = Utc::now().to_rfc3339();
    let turn_dir = committed_agent_turn_dir(&files, pending.turn_id.as_str());
    fs::create_dir_all(&turn_dir)
        .with_context(|| format!("failed to create {}", turn_dir.display()))?;
    let response_path = turn_dir.join("agent_response.json");
    write_json(&response_path, &response)?;
    commit_post_advance_materialization(
        &files,
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        "agent_context_projection",
        || {
            append_agent_context_event_plan(files.dir.as_path(), &context_event_plan)?;
            rebuild_agent_context_projection(files.dir.as_path())?;
            Ok(())
        },
    )?;
    commit_post_advance_materialization(
        &files,
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        "relationship_graph",
        || {
            append_relationship_graph_event_plan(
                files.dir.as_path(),
                &relationship_graph_event_plan,
            )?;
            rebuild_relationship_graph(
                files.dir.as_path(),
                &RelationshipGraphPacket {
                    world_id: options.world_id.clone(),
                    turn_id: pending.turn_id.clone(),
                    ..RelationshipGraphPacket::default()
                },
            )?;
            Ok(())
        },
    )?;
    commit_post_advance_materialization(
        &files,
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        "actor_agency",
        || {
            append_actor_agency_event_plan(files.dir.as_path(), &actor_agency_event_plan)?;
            rebuild_actor_agency_packet(
                files.dir.as_path(),
                &ActorAgencyPacket {
                    world_id: options.world_id.clone(),
                    turn_id: pending.turn_id.clone(),
                    ..ActorAgencyPacket::default()
                },
            )?;
            Ok(())
        },
    )?;
    commit_post_advance_materialization(
        &files,
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        "world_lore",
        || {
            append_world_lore_update_plan(files.dir.as_path(), &world_lore_update_plan)?;
            rebuild_world_lore(
                files.dir.as_path(),
                &WorldLorePacket {
                    world_id: options.world_id.clone(),
                    turn_id: pending.turn_id.clone(),
                    ..WorldLorePacket::default()
                },
            )?;
            Ok(())
        },
    )?;
    commit_post_advance_materialization(
        &files,
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        "character_text_design",
        || {
            append_character_text_design_event_plan(
                files.dir.as_path(),
                &character_text_design_event_plan,
            )?;
            rebuild_character_text_design(
                files.dir.as_path(),
                &pending.visible_context.active_character_text_design,
            )?;
            Ok(())
        },
    )?;
    commit_post_advance_materialization(
        &files,
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        "context_capsules",
        || {
            let active_world_lore = load_world_lore_state(
                files.dir.as_path(),
                WorldLorePacket {
                    world_id: options.world_id.clone(),
                    turn_id: pending.turn_id.clone(),
                    ..WorldLorePacket::default()
                },
            )?;
            let active_relationship_graph = load_relationship_graph_state(
                files.dir.as_path(),
                RelationshipGraphPacket {
                    world_id: options.world_id.clone(),
                    turn_id: pending.turn_id.clone(),
                    ..RelationshipGraphPacket::default()
                },
            )?;
            let active_character_text_design = load_character_text_design_state(
                files.dir.as_path(),
                pending.visible_context.active_character_text_design.clone(),
            )?;
            rebuild_context_capsule_registry(&ContextCapsuleBuildInput {
                world_dir: files.dir.as_path(),
                world_id: options.world_id.as_str(),
                turn_id: pending.turn_id.as_str(),
                active_world_lore: &active_world_lore,
                active_relationship_graph: &active_relationship_graph,
                active_character_text_design: &active_character_text_design,
            })?;
            Ok(())
        },
    )?;
    commit_post_advance_materialization(
        &files,
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        "change_ledger",
        || {
            append_change_event_plan(files.dir.as_path(), &change_event_plan)?;
            rebuild_change_ledger(
                files.dir.as_path(),
                &ChangeLedgerPacket {
                    world_id: options.world_id.clone(),
                    turn_id: pending.turn_id.clone(),
                    ..ChangeLedgerPacket::default()
                },
            )?;
            Ok(())
        },
    )?;
    commit_post_advance_materialization(
        &files,
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        "belief_graph",
        || {
            append_belief_event_plan(files.dir.as_path(), &belief_event_plan)?;
            rebuild_belief_graph(
                files.dir.as_path(),
                &BeliefGraphPacket {
                    world_id: options.world_id.clone(),
                    turn_id: pending.turn_id.clone(),
                    ..BeliefGraphPacket::default()
                },
            )?;
            Ok(())
        },
    )?;
    commit_post_advance_materialization(
        &files,
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        "world_process_clock",
        || {
            append_world_process_event_plan(files.dir.as_path(), &world_process_event_plan)?;
            rebuild_world_process_clock(
                files.dir.as_path(),
                &pending.visible_context.active_world_process_clock,
            )?;
            Ok(())
        },
    )?;
    commit_post_advance_materialization(
        &files,
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        "player_intent_trace",
        || {
            append_player_intent_event_plan(files.dir.as_path(), &player_intent_event_plan)?;
            rebuild_player_intent_trace(
                files.dir.as_path(),
                &PlayerIntentTracePacket {
                    world_id: options.world_id.clone(),
                    turn_id: pending.turn_id.clone(),
                    ..PlayerIntentTracePacket::default()
                },
            )?;
            Ok(())
        },
    )?;
    commit_post_advance_materialization(
        &files,
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        "narrative_style_state",
        || {
            append_narrative_style_event_plan(files.dir.as_path(), &narrative_style_event_plan)?;
            rebuild_narrative_style_state(
                files.dir.as_path(),
                &pending.visible_context.active_narrative_style_state,
            )?;
            Ok(())
        },
    )?;
    commit_post_advance_materialization(
        &files,
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        "pattern_debt",
        || {
            append_pattern_debt_event_plan(files.dir.as_path(), &pattern_debt_event_plan)?;
            rebuild_pattern_debt(
                files.dir.as_path(),
                &PatternDebtPacket {
                    world_id: options.world_id.clone(),
                    turn_id: pending.turn_id.clone(),
                    ..PatternDebtPacket::default()
                },
            )?;
            Ok(())
        },
    )?;
    commit_post_advance_materialization(
        &files,
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        "plot_threads",
        || {
            append_plot_thread_event_plan(files.dir.as_path(), &plot_thread_event_plan)?;
            rebuild_plot_threads(
                files.dir.as_path(),
                &pending.visible_context.active_plot_threads,
            )?;
            Ok(())
        },
    )?;
    commit_post_advance_materialization(
        &files,
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        "scene_pressure",
        || {
            append_scene_pressure_event_plan(files.dir.as_path(), &scene_pressure_event_plan)?;
            rebuild_active_scene_pressures(
                files.dir.as_path(),
                &pending.visible_context.active_scene_pressure,
            )?;
            Ok(())
        },
    )?;
    commit_post_advance_materialization(
        &files,
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        "scene_director",
        || {
            append_scene_director_event_plan(files.dir.as_path(), &scene_director_event_plan)?;
            rebuild_scene_director(files.dir.as_path(), &active_scene_director)?;
            Ok(())
        },
    )?;
    commit_post_advance_materialization(
        &files,
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        "consequence_spine",
        || {
            append_consequence_event_plan(files.dir.as_path(), &consequence_event_plan)?;
            rebuild_consequence_spine(
                files.dir.as_path(),
                &ConsequenceSpinePacket {
                    world_id: options.world_id.clone(),
                    turn_id: pending.turn_id.clone(),
                    ..pending.visible_context.active_consequence_spine.clone()
                },
            )?;
            Ok(())
        },
    )?;
    commit_post_advance_materialization(
        &files,
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        "social_exchange",
        || {
            append_social_exchange_event_plan(files.dir.as_path(), &social_exchange_event_plan)?;
            rebuild_social_exchange(
                files.dir.as_path(),
                &SocialExchangePacket {
                    world_id: options.world_id.clone(),
                    turn_id: pending.turn_id.clone(),
                    ..pending.visible_context.active_social_exchange.clone()
                },
            )?;
            Ok(())
        },
    )?;
    commit_post_advance_materialization(
        &files,
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        "encounter_surface",
        || {
            append_encounter_surface_event_plan(
                files.dir.as_path(),
                &encounter_surface_event_plan,
            )?;
            rebuild_encounter_surface(
                files.dir.as_path(),
                &EncounterSurfacePacket {
                    world_id: options.world_id.clone(),
                    turn_id: pending.turn_id.clone(),
                    ..pending.visible_context.active_encounter_surface.clone()
                },
            )?;
            Ok(())
        },
    )?;
    commit_post_advance_materialization(
        &files,
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        "body_resource",
        || {
            append_body_resource_event_plan(files.dir.as_path(), &body_resource_event_plan)?;
            rebuild_body_resource_state(
                files.dir.as_path(),
                &pending.visible_context.active_body_resource_state,
            )?;
            Ok(())
        },
    )?;
    commit_post_advance_materialization(
        &files,
        options.world_id.as_str(),
        pending.turn_id.as_str(),
        "location_graph",
        || {
            append_location_event_plan(files.dir.as_path(), &location_event_plan)?;
            rebuild_location_graph(
                files.dir.as_path(),
                &pending.visible_context.active_location_graph,
            )?;
            Ok(())
        },
    )?;

    let packet = build_vn_packet(&BuildVnPacketOptions {
        store_root: options.store_root.clone(),
        world_id: options.world_id.clone(),
        turn_id: Some(pending.turn_id.clone()),
        scene_image_url: None,
    })?;
    let commit_record_path = turn_dir.join(AGENT_COMMIT_RECORD_FILENAME);
    let committed = CommittedAgentTurn {
        schema_version: AGENT_COMMIT_RECORD_SCHEMA_VERSION.to_owned(),
        world_id: options.world_id.clone(),
        turn_id: pending.turn_id.clone(),
        render_packet_path: advanced.render_packet_path.display().to_string(),
        response_path: response_path.display().to_string(),
        commit_record_path: commit_record_path.display().to_string(),
        committed_at,
        packet,
    };
    write_json(&commit_record_path, &committed)?;
    append_turn_commit_envelope(
        files.dir.as_path(),
        &TurnCommitEnvelope::committed(
            &pending,
            before_commit_snapshot.turn_id.as_str(),
            &committed,
        ),
    )?;
    commit_extra_memory_projection_terminal(&extra_memory_projection)?;
    fs::remove_file(&pending_path)
        .with_context(|| format!("failed to remove {}", pending_path.display()))?;
    Ok(committed)
}

fn canonical_agent_turn_response(mut response: AgentTurnResponse) -> AgentTurnResponse {
    if response.next_choices.len() == 7 {
        response.next_choices = normalize_turn_choices(&response.next_choices);
    }
    response
}

fn commit_post_advance_materialization(
    files: &WorldFilePaths,
    world_id: &str,
    turn_id: &str,
    component: &str,
    materialize: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let event = match materialize() {
        Ok(()) => PostCommitMaterializationEvent {
            schema_version: POST_COMMIT_MATERIALIZATION_EVENT_SCHEMA_VERSION.to_owned(),
            world_id: world_id.to_owned(),
            turn_id: turn_id.to_owned(),
            component: component.to_owned(),
            status: "committed".to_owned(),
            error: None,
            recorded_at: Utc::now().to_rfc3339(),
        },
        Err(error) => PostCommitMaterializationEvent {
            schema_version: POST_COMMIT_MATERIALIZATION_EVENT_SCHEMA_VERSION.to_owned(),
            world_id: world_id.to_owned(),
            turn_id: turn_id.to_owned(),
            component: component.to_owned(),
            status: "failed".to_owned(),
            error: Some(format!("{error:#}")),
            recorded_at: Utc::now().to_rfc3339(),
        },
    };
    append_jsonl(
        &files.dir.join(POST_COMMIT_MATERIALIZATION_EVENTS_FILENAME),
        &event,
    )
}

fn validate_agent_response(pending: &PendingAgentTurn, response: &AgentTurnResponse) -> Result<()> {
    if response.schema_version != AGENT_TURN_RESPONSE_SCHEMA_VERSION {
        bail!(
            "agent response schema_version mismatch: expected {}, got {}",
            AGENT_TURN_RESPONSE_SCHEMA_VERSION,
            response.schema_version
        );
    }
    if response.world_id != pending.world_id || response.turn_id != pending.turn_id {
        bail!(
            "agent response target mismatch: pending={}/{}, response={}/{}",
            pending.world_id,
            pending.turn_id,
            response.world_id,
            response.turn_id
        );
    }
    if !response.needs_context.is_empty() {
        bail!(
            "agent response requested bounded context repair before commit: requests={}",
            response.needs_context.len()
        );
    }
    if response.visible_scene.schema_version != NARRATIVE_SCENE_SCHEMA_VERSION {
        bail!(
            "visible_scene schema_version mismatch: expected {}, got {}",
            NARRATIVE_SCENE_SCHEMA_VERSION,
            response.visible_scene.schema_version
        );
    }
    if response
        .visible_scene
        .text_blocks
        .iter()
        .all(|block| block.trim().is_empty())
    {
        bail!("agent response visible_scene.text_blocks must contain visible narrative text");
    }
    validate_agent_next_choices(response)?;
    ensure_no_hidden_leak(pending, response)
}

fn audit_agent_resolution_proposal(
    store_root: Option<&Path>,
    pending: &PendingAgentTurn,
    response: &AgentTurnResponse,
) -> Result<()> {
    let Some(proposal) = &response.resolution_proposal else {
        return Ok(());
    };
    let prompt_context = compile_prompt_context_packet(&CompilePromptContextPacketOptions {
        store_root,
        pending,
        engine_session_kind: "webgpt_project_session",
    })?;
    audit_resolution_proposal(&prompt_context, proposal)
        .map_err(|critique| resolution_critique_error(&critique))
        .and_then(|()| {
            audit_resolution_choices(&prompt_context, proposal, &response.next_choices)
                .map_err(|critique| resolution_critique_error(&critique))
        })
}

fn audit_agent_scene_director_proposal(
    store_root: Option<&Path>,
    pending: &PendingAgentTurn,
    response: &AgentTurnResponse,
) -> Result<()> {
    let Some(proposal) = &response.scene_director_proposal else {
        return Ok(());
    };
    let prompt_context = compile_prompt_context_packet(&CompilePromptContextPacketOptions {
        store_root,
        pending,
        engine_session_kind: "webgpt_project_session",
    })?;
    audit_scene_director_proposal(
        &prompt_context,
        proposal,
        response.resolution_proposal.as_ref(),
        &response.next_choices,
    )
    .map_err(|critique| scene_director_critique_error(&critique))
}

fn scene_director_critique_error(critique: &SceneDirectorCritique) -> anyhow::Error {
    anyhow::anyhow!(
        "scene director proposal audit failed: failure_kind={:?}, message={}, rejected_refs={:?}, required_changes={:?}",
        critique.failure_kind,
        critique.message,
        critique.rejected_refs,
        critique.required_changes
    )
}

fn resolution_critique_error(critique: &ResolutionCritique) -> anyhow::Error {
    anyhow::anyhow!(
        "resolution proposal audit failed: failure_kind={:?}, message={}, rejected_refs={:?}",
        critique.failure_kind,
        critique.message,
        critique.rejected_refs
    )
}

fn validate_agent_next_choices(response: &AgentTurnResponse) -> Result<()> {
    if response.next_choices.len() != 7 {
        bail!(
            "agent response next_choices must contain exactly slots 1..7: actual_len={}",
            response.next_choices.len()
        );
    }
    let slots = response
        .next_choices
        .iter()
        .map(|choice| choice.slot)
        .collect::<BTreeSet<_>>();
    if slots != BTreeSet::from([1, 2, 3, 4, 5, 6, 7]) {
        bail!("agent response next_choices must contain slots 1..7 exactly: actual={slots:?}");
    }
    let guide_choice = response
        .next_choices
        .iter()
        .find(|choice| choice.slot == GUIDE_CHOICE_SLOT)
        .context("agent response next_choices missing slot 7")?;
    if !is_guide_choice_tag(guide_choice.tag.as_str())
        || guide_choice.intent != "맡긴다. 세부 내용은 선택 후 드러난다."
    {
        bail!("agent response slot 7 must keep hidden delegated-judgment wording");
    }
    let freeform_choice = response
        .next_choices
        .iter()
        .find(|choice| choice.slot == FREEFORM_CHOICE_SLOT)
        .context("agent response next_choices missing slot 6")?;
    if freeform_choice.tag != "자유서술" || !freeform_choice.intent.contains("직접") {
        bail!("agent response slot 6 must remain inline freeform");
    }
    for choice in &response.next_choices {
        validate_player_visible_choice_text(choice)?;
    }
    if choices_keep_default_template(&response.next_choices) {
        bail!(
            "agent response next_choices must be scene-specific; default template choices leaked"
        );
    }
    Ok(())
}

fn validate_player_visible_choice_text(choice: &TurnChoice) -> Result<()> {
    for (field, value) in [
        ("tag", choice.tag.as_str()),
        ("intent", choice.intent.as_str()),
    ] {
        if leaks_internal_choice_token(value) {
            bail!(
                "agent response next_choices[slot={}] {field} contains internal token",
                choice.slot
            );
        }
    }
    Ok(())
}

fn leaks_internal_choice_token(value: &str) -> bool {
    [
        "char:anchor",
        "anchor_character",
        "앵커 인물",
        "hidden",
        "secret",
        "숨겨진 진실",
        "시드가 정한",
        "seed-defined",
    ]
    .iter()
    .any(|needle| value.contains(needle))
}

fn choices_keep_default_template(choices: &[TurnChoice]) -> bool {
    let defaults = default_turn_choices();
    [1, 2, 3, 4, 5].iter().all(|slot| {
        let Some(choice) = choices.iter().find(|choice| choice.slot == *slot) else {
            return false;
        };
        let Some(default_choice) = defaults.iter().find(|choice| choice.slot == *slot) else {
            return false;
        };
        choice.tag == default_choice.tag && choice.intent == default_choice.intent
    })
}

fn ensure_no_hidden_leak(pending: &PendingAgentTurn, response: &AgentTurnResponse) -> Result<()> {
    let visible_text = response.visible_scene.text_blocks.join("\n");
    for secret in &pending.private_adjudication_context.unrevealed_constraints {
        reject_visible_needle(
            visible_text.as_str(),
            secret.truth.as_str(),
            secret.secret_id.as_str(),
        )?;
        for forbidden in &secret.forbidden_leaks {
            reject_visible_needle(
                visible_text.as_str(),
                forbidden.as_str(),
                secret.secret_id.as_str(),
            )?;
        }
    }
    Ok(())
}

fn reject_visible_needle(visible_text: &str, needle: &str, secret_id: &str) -> Result<()> {
    let needle = needle.trim();
    if needle.chars().count() < 4 {
        return Ok(());
    }
    if visible_text.contains(needle) {
        bail!("agent response leaks hidden truth: secret_id={secret_id}");
    }
    Ok(())
}

fn apply_agent_response_to_render_packet(
    packet: &mut crate::models::RenderPacket,
    response: &AgentTurnResponse,
) {
    packet.narrative_scene = Some(response.visible_scene.clone());
    if let Some(adjudication) = &response.adjudication {
        if let Some(packet_adjudication) = packet.adjudication.as_mut() {
            packet_adjudication
                .outcome
                .clone_from(&adjudication.outcome);
            packet_adjudication
                .summary
                .clone_from(&adjudication.summary);
            packet_adjudication.gates.clone_from(&adjudication.gates);
            packet_adjudication
                .visible_constraints
                .clone_from(&adjudication.visible_constraints);
            packet_adjudication
                .consequences
                .clone_from(&adjudication.consequences);
        }
        packet
            .visible_state
            .dashboard
            .status
            .clone_from(&adjudication.summary);
    }
    if let Some(canon_event) = &response.canon_event {
        packet
            .visible_state
            .dashboard
            .status
            .clone_from(&canon_event.summary);
    }
    packet.visible_state.choices = normalize_turn_choices(&response.next_choices);
}

fn persist_agent_next_choices(
    files: &WorldFilePaths,
    snapshot_path: &Path,
    snapshot: &TurnSnapshot,
    choices: &[TurnChoice],
) -> Result<()> {
    let mut updated = snapshot.clone();
    updated.last_choices = normalize_turn_choices(choices);
    write_json(snapshot_path, &updated)?;
    write_json(&files.latest_snapshot, &updated)
}

fn selected_choice(input: &str, snapshot: &TurnSnapshot) -> Option<PendingAgentChoice> {
    let choices = normalize_turn_choices(&snapshot.last_choices);
    let choice =
        numeric_choice(input, &choices).or_else(|| inline_freeform_choice(input, &choices));
    choice.map(|choice| {
        let visible_intent = choice.player_visible_intent().to_owned();
        PendingAgentChoice {
            slot: choice.slot,
            tag: choice.tag,
            visible_intent,
        }
    })
}

fn numeric_choice(input: &str, choices: &[TurnChoice]) -> Option<TurnChoice> {
    let slot = input.trim().parse::<u8>().ok()?;
    choices.iter().find(|choice| choice.slot == slot).cloned()
}

fn inline_freeform_choice(input: &str, choices: &[TurnChoice]) -> Option<TurnChoice> {
    let slot_digit = char::from_digit(u32::from(FREEFORM_CHOICE_SLOT), 10)?;
    let rest = input.trim().strip_prefix(slot_digit)?;
    if !(rest.starts_with("번")
        || rest.starts_with(char::is_whitespace)
        || rest
            .chars()
            .next()
            .is_some_and(|ch| matches!(ch, '.' | ')' | ':' | '-' | '—')))
    {
        return None;
    }
    choices
        .iter()
        .find(|choice| choice.slot == FREEFORM_CHOICE_SLOT)
        .cloned()
        .or_else(|| Some(default_freeform_choice()))
}

struct VisibleContextInput<'a> {
    files: &'a WorldFilePaths,
    snapshot: &'a TurnSnapshot,
    entities: &'a crate::models::EntityRecords,
    current_packet: &'a VnPacket,
    selected_choice: Option<&'a PendingAgentChoice>,
    private_context: &'a AgentPrivateAdjudicationContext,
    player_input: &'a str,
    output_contract: &'a AgentOutputContract,
}

struct ActiveProjectionContext {
    change_ledger: ChangeLedgerPacket,
    pattern_debt: PatternDebtPacket,
    belief_graph: BeliefGraphPacket,
    world_process_clock: WorldProcessClockPacket,
    actor_agency: ActorAgencyPacket,
    player_intent_trace: PlayerIntentTracePacket,
    narrative_style_state: NarrativeStyleState,
}

struct VisibleTurnRetrievalSource<'a> {
    plot_threads: &'a PlotThreadPacket,
    scene_pressure: &'a ScenePressurePacket,
    relationship_graph: &'a RelationshipGraphPacket,
    body_resource_state: &'a BodyResourcePacket,
    location_graph: &'a LocationGraphPacket,
    projections: &'a ActiveProjectionContext,
}

#[allow(clippy::too_many_lines)]
fn visible_context(input: &VisibleContextInput<'_>) -> Result<AgentVisibleContext> {
    let files = input.files;
    let snapshot = input.snapshot;
    let next_turn_id = next_turn_id(snapshot.turn_id.as_str())?;
    let extra_memory = retrieve_extra_memory_packet(
        files.dir.as_path(),
        input.current_packet.world_id.as_str(),
        snapshot,
        input.player_input,
    )?;
    let active_consequence_spine = load_consequence_spine_state(
        files.dir.as_path(),
        ConsequenceSpinePacket {
            world_id: input.current_packet.world_id.clone(),
            turn_id: next_turn_id.clone(),
            ..ConsequenceSpinePacket::default()
        },
    )?;
    let active_social_exchange = load_social_exchange_state(
        files.dir.as_path(),
        SocialExchangePacket {
            world_id: input.current_packet.world_id.clone(),
            turn_id: next_turn_id.clone(),
            ..SocialExchangePacket::default()
        },
    )?;
    let active_scene_pressure = merge_social_exchange_scene_pressures(
        merge_consequence_scene_pressures(
            load_active_scene_pressures(
                files.dir.as_path(),
                compile_scene_pressure_packet(
                    snapshot,
                    input.selected_choice,
                    &extra_memory,
                    input.private_context,
                    input.player_input,
                )?,
            )?,
            &active_consequence_spine,
        ),
        &active_social_exchange,
    );
    let active_plot_threads =
        load_plot_threads(files.dir.as_path(), compile_plot_thread_packet(snapshot))?;
    let active_body_resource_state =
        load_body_resource_prompt_packet(files.dir.as_path(), snapshot)?;
    let active_location_graph = load_location_graph_state(
        files.dir.as_path(),
        compile_location_graph_packet(snapshot, input.entities),
    )?;
    let context_projection = load_agent_context_projection(files.dir.as_path())?;
    let active_character_text_design = load_character_text_design_state(
        files.dir.as_path(),
        compile_character_text_design_with_projection(input.entities, &context_projection),
    )?;
    let active_world_lore =
        load_visible_world_lore(input, next_turn_id.as_str(), &context_projection)?;
    let active_relationship_graph =
        load_visible_relationship_graph(input, next_turn_id.as_str(), &context_projection)?;
    let mut active_projections = load_active_projection_context(input, next_turn_id.as_str())?;
    active_projections.actor_agency =
        merge_consequence_actor_agency(active_projections.actor_agency, &active_consequence_spine);
    active_projections.actor_agency = merge_social_exchange_actor_agency(
        active_projections.actor_agency,
        &active_social_exchange,
    );
    active_projections.world_process_clock = merge_consequence_world_processes(
        active_projections.world_process_clock,
        &active_consequence_spine,
    );
    let active_turn_retrieval_controller = compile_visible_turn_retrieval_controller(
        input,
        next_turn_id.as_str(),
        &VisibleTurnRetrievalSource {
            plot_threads: &active_plot_threads,
            scene_pressure: &active_scene_pressure,
            relationship_graph: &active_relationship_graph,
            body_resource_state: &active_body_resource_state,
            location_graph: &active_location_graph,
            projections: &active_projections,
        },
    )?;
    let context_capsules = rebuild_and_select_visible_context_capsules(
        input,
        next_turn_id.as_str(),
        &active_world_lore,
        &active_relationship_graph,
        &active_character_text_design,
        &active_turn_retrieval_controller,
    )?;
    let active_autobiographical_index = rebuild_visible_autobiographical_index(
        input,
        next_turn_id.as_str(),
        &active_plot_threads,
        &active_relationship_graph,
        &active_projections.world_process_clock,
        &context_capsules.index,
    )?;
    let pattern_debt_value = serde_json::to_value(&active_projections.pattern_debt)?;
    let plot_threads_value = serde_json::to_value(&active_plot_threads)?;
    let actor_agency_value = serde_json::to_value(&active_projections.actor_agency)?;
    let world_process_clock_value = serde_json::to_value(&active_projections.world_process_clock)?;
    let player_intent_trace_value = serde_json::to_value(&active_projections.player_intent_trace)?;
    let active_social_exchange_value = serde_json::to_value(&active_social_exchange)?;
    let recent_scene_window_value = serde_json::to_value(&input.current_packet.scene.text_blocks)?;
    let compiled_encounter_surface = compile_encounter_surface_packet(
        input.current_packet.world_id.as_str(),
        next_turn_id.as_str(),
        snapshot.protagonist_state.location.as_str(),
        &snapshot.last_choices,
        &active_scene_pressure,
        &active_location_graph,
        &active_body_resource_state,
        &active_social_exchange,
    );
    let active_encounter_surface =
        load_encounter_surface_state(files.dir.as_path(), compiled_encounter_surface)?;
    let active_encounter_surface_value = serde_json::to_value(&active_encounter_surface)?;
    let compiled_scene_director =
        compile_scene_director_packet_from_input(SceneDirectorCompileInput {
            world_id: input.current_packet.world_id.as_str(),
            turn_id: next_turn_id.as_str(),
            scene_pressure: &active_scene_pressure,
            active_pattern_debt: &pattern_debt_value,
            active_plot_threads: Some(&plot_threads_value),
            active_actor_agency: Some(&actor_agency_value),
            active_world_process_clock: Some(&world_process_clock_value),
            active_player_intent_trace: Some(&player_intent_trace_value),
            active_social_exchange: Some(&active_social_exchange_value),
            active_encounter_surface: Some(&active_encounter_surface_value),
            recent_scene_window: Some(&recent_scene_window_value),
        });
    let active_scene_director = merge_scene_director_history(
        compiled_scene_director,
        Some(&load_scene_director_state(
            files.dir.as_path(),
            SceneDirectorPacket::default(),
        )?),
    );
    Ok(AgentVisibleContext {
        location: snapshot.protagonist_state.location.clone(),
        recent_scene: input.current_packet.scene.text_blocks.clone(),
        known_facts: known_facts(snapshot),
        voice_anchors: input
            .entities
            .characters
            .iter()
            .filter(|character| !character.voice_anchor.is_empty())
            .map(|character| AgentVoiceAnchor {
                character_id: character.id.clone(),
                name: character.name.visible.clone(),
                anchor: character.voice_anchor.clone(),
            })
            .collect(),
        extra_memory,
        active_scene_pressure,
        active_plot_threads,
        active_body_resource_state,
        active_location_graph,
        active_character_text_design,
        active_world_lore,
        active_relationship_graph,
        active_actor_agency: active_projections.actor_agency,
        active_change_ledger: active_projections.change_ledger,
        active_pattern_debt: active_projections.pattern_debt,
        active_belief_graph: active_projections.belief_graph,
        active_world_process_clock: active_projections.world_process_clock,
        active_player_intent_trace: active_projections.player_intent_trace,
        active_narrative_style_state: active_projections.narrative_style_state,
        active_scene_director,
        active_consequence_spine,
        active_social_exchange,
        active_encounter_surface,
        active_turn_retrieval_controller,
        selected_context_capsules: context_capsules.selection,
        active_autobiographical_index,
    })
}

fn compile_visible_turn_retrieval_controller(
    input: &VisibleContextInput<'_>,
    turn_id: &str,
    source: &VisibleTurnRetrievalSource<'_>,
) -> Result<TurnRetrievalControllerPacket> {
    compile_turn_retrieval_controller(&TurnRetrievalCompileInput {
        world_dir: input.files.dir.as_path(),
        world_id: input.current_packet.world_id.as_str(),
        turn_id,
        snapshot: input.snapshot,
        player_input: input.player_input,
        active_plot_threads: source.plot_threads,
        active_scene_pressure: source.scene_pressure,
        active_relationship_graph: source.relationship_graph,
        active_body_resource_state: source.body_resource_state,
        active_location_graph: source.location_graph,
        active_world_process_clock: &source.projections.world_process_clock,
        active_player_intent_trace: &source.projections.player_intent_trace,
    })
}

fn load_visible_world_lore(
    input: &VisibleContextInput<'_>,
    turn_id: &str,
    context_projection: &crate::response_context::AgentContextProjection,
) -> Result<WorldLorePacket> {
    load_world_lore_state(
        input.files.dir.as_path(),
        compile_world_lore_from_projection(
            input.current_packet.world_id.as_str(),
            turn_id,
            context_projection,
            &[],
        ),
    )
}

fn load_visible_relationship_graph(
    input: &VisibleContextInput<'_>,
    turn_id: &str,
    context_projection: &crate::response_context::AgentContextProjection,
) -> Result<RelationshipGraphPacket> {
    load_relationship_graph_state(
        input.files.dir.as_path(),
        compile_relationship_graph_from_projection(
            input.current_packet.world_id.as_str(),
            turn_id,
            context_projection,
            &[],
        ),
    )
}

struct VisibleContextCapsules {
    index: ContextCapsuleIndex,
    selection: ContextCapsuleSelection,
}

fn rebuild_and_select_visible_context_capsules(
    input: &VisibleContextInput<'_>,
    turn_id: &str,
    active_world_lore: &WorldLorePacket,
    active_relationship_graph: &RelationshipGraphPacket,
    active_character_text_design: &CharacterTextDesignPacket,
    active_turn_retrieval_controller: &TurnRetrievalControllerPacket,
) -> Result<VisibleContextCapsules> {
    let index = rebuild_context_capsule_registry(&ContextCapsuleBuildInput {
        world_dir: input.files.dir.as_path(),
        world_id: input.current_packet.world_id.as_str(),
        turn_id,
        active_world_lore,
        active_relationship_graph,
        active_character_text_design,
    })?;
    let selection = select_context_capsules(&ContextCapsuleSelectionInput {
        world_dir: input.files.dir.as_path(),
        world_id: input.current_packet.world_id.as_str(),
        turn_id,
        player_input: input.player_input,
        retrieval_controller: active_turn_retrieval_controller,
    })?;
    Ok(VisibleContextCapsules { index, selection })
}

fn rebuild_visible_autobiographical_index(
    input: &VisibleContextInput<'_>,
    turn_id: &str,
    active_plot_threads: &PlotThreadPacket,
    active_relationship_graph: &RelationshipGraphPacket,
    active_world_process_clock: &WorldProcessClockPacket,
    context_capsule_index: &ContextCapsuleIndex,
) -> Result<AutobiographicalIndexPacket> {
    let chapter_summaries = latest_chapter_summaries(
        input.files.dir.as_path(),
        input.current_packet.world_id.as_str(),
        8,
    )?;
    rebuild_autobiographical_index(&AutobiographicalIndexInput {
        world_dir: input.files.dir.as_path(),
        world_id: input.current_packet.world_id.as_str(),
        turn_id,
        chapter_summaries: &chapter_summaries,
        plot_threads: active_plot_threads,
        relationship_graph: active_relationship_graph,
        world_process_clock: active_world_process_clock,
        context_capsule_index,
    })
}

fn load_active_projection_context(
    input: &VisibleContextInput<'_>,
    turn_id: &str,
) -> Result<ActiveProjectionContext> {
    let world_id = input.current_packet.world_id.as_str();
    Ok(ActiveProjectionContext {
        change_ledger: load_change_ledger_state(
            input.files.dir.as_path(),
            ChangeLedgerPacket {
                world_id: world_id.to_owned(),
                turn_id: turn_id.to_owned(),
                ..ChangeLedgerPacket::default()
            },
        )?,
        pattern_debt: load_pattern_debt_state(
            input.files.dir.as_path(),
            PatternDebtPacket {
                world_id: world_id.to_owned(),
                turn_id: turn_id.to_owned(),
                ..PatternDebtPacket::default()
            },
        )?,
        belief_graph: load_belief_graph_state(
            input.files.dir.as_path(),
            BeliefGraphPacket {
                world_id: world_id.to_owned(),
                turn_id: turn_id.to_owned(),
                ..BeliefGraphPacket::default()
            },
        )?,
        world_process_clock: load_world_process_clock_state(
            input.files.dir.as_path(),
            WorldProcessClockPacket {
                world_id: world_id.to_owned(),
                turn_id: turn_id.to_owned(),
                ..WorldProcessClockPacket::default()
            },
        )?,
        actor_agency: load_actor_agency_state(
            input.files.dir.as_path(),
            ActorAgencyPacket {
                world_id: world_id.to_owned(),
                turn_id: turn_id.to_owned(),
                ..ActorAgencyPacket::default()
            },
        )?,
        player_intent_trace: load_player_intent_trace_state(
            input.files.dir.as_path(),
            PlayerIntentTracePacket {
                world_id: world_id.to_owned(),
                turn_id: turn_id.to_owned(),
                ..PlayerIntentTracePacket::default()
            },
        )?,
        narrative_style_state: load_narrative_style_state(
            input.files.dir.as_path(),
            compile_narrative_style_state(world_id, turn_id, input.output_contract),
        )?,
    })
}

fn known_facts(snapshot: &TurnSnapshot) -> Vec<String> {
    let mut facts = Vec::new();
    facts.extend(
        snapshot
            .open_questions
            .iter()
            .map(|question| format!("open_question: {question}")),
    );
    facts.extend(
        snapshot
            .protagonist_state
            .mind
            .iter()
            .map(|mind| format!("mind: {mind}")),
    );
    facts.extend(
        snapshot
            .protagonist_state
            .body
            .iter()
            .map(|body| format!("body: {body}")),
    );
    if let Some(event) = &snapshot.current_event {
        facts.push(format!(
            "current_event: {} / {}",
            event.event_id, event.progress
        ));
    }
    facts
}

fn private_context(hidden_state: &HiddenState) -> AgentPrivateAdjudicationContext {
    AgentPrivateAdjudicationContext {
        hidden_timers: hidden_state
            .timers
            .iter()
            .map(|timer| AgentHiddenTimer {
                timer_id: timer.timer_id.clone(),
                kind: timer.kind.clone(),
                remaining_turns: timer.remaining_turns,
                effect: timer.effect.clone(),
            })
            .collect(),
        unrevealed_constraints: hidden_state
            .secrets
            .iter()
            .map(|secret| AgentHiddenSecret {
                secret_id: secret.secret_id.clone(),
                status: secret.status.clone(),
                truth: secret.truth.clone(),
                reveal_conditions: secret.reveal_conditions.clone(),
                forbidden_leaks: secret.forbidden_leaks.clone(),
            })
            .collect(),
        plausibility_gates: ["body", "resource", "time", "social_permission", "knowledge"]
            .iter()
            .map(|gate| (*gate).to_owned())
            .collect(),
    }
}

fn next_turn_id(turn_id: &str) -> Result<String> {
    let number = turn_id
        .strip_prefix("turn_")
        .context("turn_id must start with turn_")?
        .parse::<u32>()
        .with_context(|| format!("turn_id has invalid numeric suffix: {turn_id}"))?;
    Ok(format!("turn_{:04}", number + 1))
}

fn pending_agent_turn_path(files: &WorldFilePaths) -> PathBuf {
    files
        .dir
        .join(AGENT_BRIDGE_DIR)
        .join(PENDING_AGENT_TURN_FILENAME)
}

fn committed_agent_turn_dir(files: &WorldFilePaths, turn_id: &str) -> PathBuf {
    files
        .dir
        .join(AGENT_BRIDGE_DIR)
        .join(COMMITTED_AGENT_TURNS_DIR)
        .join(turn_id)
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        AGENT_PENDING_TURN_SCHEMA_VERSION, AGENT_TURN_RESPONSE_SCHEMA_VERSION,
        AgentCommitTurnOptions, AgentContextRepairRequest, AgentExtraContact, AgentOutputContract,
        AgentPrivateAdjudicationContext, AgentSubmitTurnOptions, AgentTurnResponse,
        AgentVisibleContext, PendingAgentTurn, canonical_agent_turn_response, enqueue_agent_turn,
        load_pending_agent_turn, narrative_budget_for_level, normalize_narrative_level,
        selected_choice, validate_agent_next_choices, validate_agent_response,
    };
    use crate::actor_agency::ActorAgencyPacket;
    use crate::agent_bridge::commit_agent_turn;
    use crate::autobiographical_index::AutobiographicalIndexPacket;
    use crate::belief_graph::BeliefGraphPacket;
    use crate::body_resource::BodyResourcePacket;
    use crate::change_ledger::ChangeLedgerPacket;
    use crate::character_text_design::CharacterTextDesignPacket;
    use crate::consequence_spine::ConsequenceSpinePacket;
    use crate::context_capsule::ContextCapsuleSelection;
    use crate::encounter_surface::EncounterSurfacePacket;
    use crate::extra_memory::{
        ExtraMemoryPacket, ExtraMemoryProjectionStatus, load_extra_memory_projection_records,
        load_remembered_extras,
    };
    use crate::location_graph::LocationGraphPacket;
    use crate::models::{
        GUIDE_CHOICE_TAG, NARRATIVE_SCENE_SCHEMA_VERSION, NarrativeScene, TurnChoice, TurnSnapshot,
        default_turn_choices,
    };
    use crate::narrative_style_state::NarrativeStyleState;
    use crate::pattern_debt::PatternDebtPacket;
    use crate::player_intent::PlayerIntentTracePacket;
    use crate::plot_thread::PlotThreadPacket;
    use crate::relationship_graph::RelationshipGraphPacket;
    use crate::resolution::{
        ActionAmbiguity, ActionInputKind, ActionIntent, NarrativeBrief, ProposedEffect,
        ProposedEffectKind, RESOLUTION_PROPOSAL_SCHEMA_VERSION, ResolutionOutcome,
        ResolutionOutcomeKind, ResolutionProposal, ResolutionVisibility,
    };
    use crate::scene_director::SceneDirectorPacket;
    use crate::scene_pressure::ScenePressurePacket;
    use crate::social_exchange::SocialExchangePacket;
    use crate::store::{
        InitWorldOptions, init_world, read_json, resolve_store_paths, world_file_paths,
    };
    use crate::turn_commit::{TURN_COMMITS_FILENAME, TurnCommitEnvelope, TurnCommitStatus};
    use crate::turn_retrieval_controller::TurnRetrievalControllerPacket;
    use crate::vn::{BuildVnPacketOptions, build_vn_packet};
    use crate::world_lore::WorldLorePacket;
    use crate::world_process_clock::WorldProcessClockPacket;
    use tempfile::tempdir;

    fn seed_body(world_id: &str) -> String {
        format!(
            r#"
schema_version: singulari.world_seed.v1
world_id: {world_id}
title: "agent bridge test"
premise:
  genre: "fantasy"
  protagonist: "modern reincarnated protagonist"
"#
        )
    }

    fn scene_specific_choices() -> Vec<TurnChoice> {
        vec![
            TurnChoice {
                slot: 1,
                tag: "단서".to_owned(),
                intent: "이번 장면에서 새로 드러난 단서를 따라 조심스럽게 움직인다".to_owned(),
            },
            TurnChoice {
                slot: 2,
                tag: "몸 상태".to_owned(),
                intent: "현재 몸 상태와 주변 조건이 가능한 행동을 얼마나 제한하는지 살핀다"
                    .to_owned(),
            },
            TurnChoice {
                slot: 3,
                tag: "낮은 부름".to_owned(),
                intent: "가까운 수풀 뒤쪽에 사람이 있는지 낮은 목소리로 확인한다".to_owned(),
            },
            TurnChoice {
                slot: 4,
                tag: "기록".to_owned(),
                intent: "방금 본 단서의 의미를 세계 기록에서 대조한다".to_owned(),
            },
            TurnChoice {
                slot: 5,
                tag: "먼 시야".to_owned(),
                intent: "이 장소를 둘러싼 변화 압력을 더 넓게 본다".to_owned(),
            },
            TurnChoice {
                slot: 6,
                tag: "자유서술".to_owned(),
                intent: "6 뒤에 직접 행동, 말, 내면 독백을 서술한다".to_owned(),
            },
            TurnChoice {
                slot: 7,
                tag: GUIDE_CHOICE_TAG.to_owned(),
                intent: "맡긴다. 세부 내용은 선택 후 드러난다.".to_owned(),
            },
        ]
    }

    fn invalid_resolution_proposal(world_id: &str, turn_id: &str) -> ResolutionProposal {
        ResolutionProposal {
            schema_version: RESOLUTION_PROPOSAL_SCHEMA_VERSION.to_owned(),
            world_id: world_id.to_owned(),
            turn_id: turn_id.to_owned(),
            interpreted_intent: ActionIntent {
                input_kind: ActionInputKind::PresentedChoice,
                summary: "플레이어가 현재 장면의 첫 번째 선택지를 따른다.".to_owned(),
                target_refs: Vec::new(),
                pressure_refs: Vec::new(),
                evidence_refs: vec!["current_turn".to_owned()],
                ambiguity: ActionAmbiguity::Clear,
            },
            outcome: ResolutionOutcome {
                kind: ResolutionOutcomeKind::Success,
                summary: "장면이 이어진다.".to_owned(),
                evidence_refs: vec!["current_turn".to_owned()],
            },
            gate_results: Vec::new(),
            proposed_effects: vec![ProposedEffect {
                effect_kind: ProposedEffectKind::BodyResourceDelta,
                target_ref: "resource:missing_map".to_owned(),
                visibility: ResolutionVisibility::PlayerVisible,
                summary: "없는 지도를 사용한다.".to_owned(),
                evidence_refs: vec!["current_turn".to_owned()],
            }],
            process_ticks: Vec::new(),
            narrative_brief: NarrativeBrief {
                visible_summary: "장면이 이어진다.".to_owned(),
                required_beats: Vec::new(),
                forbidden_visible_details: Vec::new(),
            },
            next_choice_plan: Vec::new(),
        }
    }

    fn legacy_slot_contract_choices() -> Vec<TurnChoice> {
        let mut choices = scene_specific_choices();
        choices[3] = TurnChoice {
            slot: 4,
            tag: GUIDE_CHOICE_TAG.to_owned(),
            intent: "맡긴다. 세부 내용은 선택 후 드러난다.".to_owned(),
        };
        choices[4] = TurnChoice {
            slot: 5,
            tag: "기록".to_owned(),
            intent: "현재 알려진 세계 기록을 연다".to_owned(),
        };
        choices[5] = TurnChoice {
            slot: 6,
            tag: "흐름".to_owned(),
            intent: "시간의 관찰자 시점으로 다음 흐름을 본다".to_owned(),
        };
        choices[6] = TurnChoice {
            slot: 7,
            tag: "자유서술".to_owned(),
            intent: "7 뒤에 직접 행동, 말, 내면 독백을 서술한다".to_owned(),
        };
        choices
    }

    #[test]
    fn canonicalizes_legacy_slot_contract_agent_response() {
        let response = AgentTurnResponse {
            schema_version: AGENT_TURN_RESPONSE_SCHEMA_VERSION.to_owned(),
            world_id: "stw_legacy_contract".to_owned(),
            turn_id: "turn_0001".to_owned(),
            resolution_proposal: None,
            scene_director_proposal: None,
            consequence_proposal: None,
            social_exchange_proposal: None,
            encounter_proposal: None,
            visible_scene: NarrativeScene {
                schema_version: NARRATIVE_SCENE_SCHEMA_VERSION.to_owned(),
                speaker: None,
                text_blocks: vec!["agent-authored visible scene".to_owned()],
                tone_notes: Vec::new(),
            },
            adjudication: None,
            canon_event: None,
            entity_updates: Vec::new(),
            relationship_updates: Vec::new(),
            plot_thread_events: Vec::new(),
            scene_pressure_events: Vec::new(),
            world_lore_updates: Vec::new(),
            character_text_design_updates: Vec::new(),
            body_resource_events: Vec::new(),
            location_events: Vec::new(),
            extra_contacts: Vec::new(),
            hidden_state_delta: Vec::new(),
            needs_context: Vec::new(),
            next_choices: legacy_slot_contract_choices(),
            actor_goal_events: Vec::new(),
            actor_move_events: Vec::new(),
        };

        let canonical = canonical_agent_turn_response(response);

        assert_eq!(canonical.next_choices[3].slot, 4);
        assert_eq!(canonical.next_choices[3].tag, "기록");
        assert_eq!(canonical.next_choices[4].slot, 5);
        assert_eq!(canonical.next_choices[4].tag, "흐름");
        assert_eq!(canonical.next_choices[5].slot, 6);
        assert_eq!(canonical.next_choices[5].tag, "자유서술");
        assert!(canonical.next_choices[5].intent.contains("6 뒤에"));
        assert_eq!(canonical.next_choices[6].slot, 7);
        assert_eq!(canonical.next_choices[6].tag, GUIDE_CHOICE_TAG);
    }

    #[test]
    fn needs_context_request_rejects_before_commit_contract() {
        let pending = PendingAgentTurn {
            schema_version: AGENT_PENDING_TURN_SCHEMA_VERSION.to_owned(),
            world_id: "stw_needs_context".to_owned(),
            turn_id: "turn_0001".to_owned(),
            status: "pending".to_owned(),
            player_input: "문 앞의 낡은 표식을 읽는다".to_owned(),
            selected_choice: None,
            visible_context: AgentVisibleContext {
                location: "gate".to_owned(),
                recent_scene: Vec::new(),
                known_facts: Vec::new(),
                voice_anchors: Vec::new(),
                extra_memory: ExtraMemoryPacket::default(),
                active_scene_pressure: ScenePressurePacket::default(),
                active_plot_threads: PlotThreadPacket::default(),
                active_body_resource_state: BodyResourcePacket::default(),
                active_location_graph: LocationGraphPacket::default(),
                active_character_text_design: CharacterTextDesignPacket::default(),
                active_world_lore: WorldLorePacket::default(),
                active_relationship_graph: RelationshipGraphPacket::default(),
                active_actor_agency: ActorAgencyPacket::default(),
                active_change_ledger: ChangeLedgerPacket::default(),
                active_pattern_debt: PatternDebtPacket::default(),
                active_belief_graph: BeliefGraphPacket::default(),
                active_world_process_clock: WorldProcessClockPacket::default(),
                active_player_intent_trace: PlayerIntentTracePacket::default(),
                active_narrative_style_state: NarrativeStyleState::default(),
                active_scene_director: SceneDirectorPacket::default(),
                active_consequence_spine: ConsequenceSpinePacket::default(),
                active_social_exchange: SocialExchangePacket::default(),
                active_encounter_surface: EncounterSurfacePacket::default(),
                active_turn_retrieval_controller: TurnRetrievalControllerPacket::default(),
                selected_context_capsules: ContextCapsuleSelection::default(),
                active_autobiographical_index: AutobiographicalIndexPacket::default(),
            },
            private_adjudication_context: AgentPrivateAdjudicationContext {
                hidden_timers: Vec::new(),
                unrevealed_constraints: Vec::new(),
                plausibility_gates: Vec::new(),
            },
            output_contract: AgentOutputContract {
                language: "ko".to_owned(),
                must_return_json: true,
                hidden_truth_must_not_appear_in_visible_text: true,
                narrative_level: 1,
                narrative_budget: narrative_budget_for_level(Some(1)),
            },
            pending_ref: "agent_bridge/pending_turn.json".to_owned(),
            created_at: "2026-04-29T00:00:00Z".to_owned(),
        };
        let response = AgentTurnResponse {
            schema_version: AGENT_TURN_RESPONSE_SCHEMA_VERSION.to_owned(),
            world_id: "stw_needs_context".to_owned(),
            turn_id: "turn_0001".to_owned(),
            resolution_proposal: None,
            scene_director_proposal: None,
            consequence_proposal: None,
            social_exchange_proposal: None,
            encounter_proposal: None,
            visible_scene: NarrativeScene {
                schema_version: NARRATIVE_SCENE_SCHEMA_VERSION.to_owned(),
                speaker: None,
                text_blocks: vec!["장면을 닫지 않는다.".to_owned()],
                tone_notes: Vec::new(),
            },
            adjudication: None,
            canon_event: None,
            entity_updates: Vec::new(),
            relationship_updates: Vec::new(),
            plot_thread_events: Vec::new(),
            scene_pressure_events: Vec::new(),
            world_lore_updates: Vec::new(),
            character_text_design_updates: Vec::new(),
            body_resource_events: Vec::new(),
            location_events: Vec::new(),
            extra_contacts: Vec::new(),
            hidden_state_delta: Vec::new(),
            needs_context: vec![AgentContextRepairRequest {
                request_id: "needs_context:turn_0001:ash_mark".to_owned(),
                capsule_kinds: vec!["world_lore".to_owned()],
                query: "ash mark custom".to_owned(),
                reason: "selected capsules do not explain the visible mark".to_owned(),
                evidence_refs: vec![
                    "prompt_context.visible_context.selected_context_capsules".to_owned(),
                ],
            }],
            next_choices: scene_specific_choices(),
            actor_goal_events: Vec::new(),
            actor_move_events: Vec::new(),
        };

        let Err(error) = validate_agent_response(&pending, &response) else {
            panic!("needs_context must reject before turn commit");
        };
        assert!(
            error
                .to_string()
                .contains("requested bounded context repair before commit")
        );
    }

    #[test]
    fn selected_choice_interprets_legacy_snapshot_with_current_slots() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body("stw_legacy_snapshot"))?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let store_paths = resolve_store_paths(Some(store.as_path()))?;
        let files = world_file_paths(&store_paths, "stw_legacy_snapshot");
        let mut snapshot: TurnSnapshot = read_json(&files.latest_snapshot)?;
        snapshot.last_choices = legacy_slot_contract_choices();

        let Some(guide) = selected_choice("7", &snapshot) else {
            anyhow::bail!("slot 7 should map to guide");
        };
        assert_eq!(guide.slot, 7);
        assert_eq!(guide.tag, GUIDE_CHOICE_TAG);
        let Some(freeform) = selected_choice("6 문 아래의 흙을 살핀다", &snapshot) else {
            anyhow::bail!("slot 6 should map to freeform");
        };
        assert_eq!(freeform.slot, 6);
        assert_eq!(freeform.tag, "자유서술");
        Ok(())
    }

    #[test]
    fn commits_agent_scene_into_vn_packet() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body("stw_agent"))?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;

        let pending = enqueue_agent_turn(&AgentSubmitTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_agent".to_owned(),
            input: "1".to_owned(),
            narrative_level: None,
        })?;
        assert_eq!(pending.output_contract.narrative_level, 1);
        let committed = commit_agent_turn(&AgentCommitTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_agent".to_owned(),
            response: AgentTurnResponse {
                schema_version: AGENT_TURN_RESPONSE_SCHEMA_VERSION.to_owned(),
                world_id: pending.world_id.clone(),
                turn_id: pending.turn_id.clone(),
                resolution_proposal: None,
                scene_director_proposal: None,
                consequence_proposal: None,
                social_exchange_proposal: None,
                encounter_proposal: None,
                visible_scene: NarrativeScene {
                    schema_version: NARRATIVE_SCENE_SCHEMA_VERSION.to_owned(),
                    speaker: None,
                    text_blocks: vec!["agent-authored visible scene".to_owned()],
                    tone_notes: vec!["test".to_owned()],
                },
                adjudication: None,
                canon_event: None,
                entity_updates: Vec::new(),
                relationship_updates: Vec::new(),
                plot_thread_events: Vec::new(),
                scene_pressure_events: Vec::new(),
                world_lore_updates: Vec::new(),
                character_text_design_updates: Vec::new(),
                body_resource_events: Vec::new(),
                location_events: Vec::new(),
                extra_contacts: Vec::new(),
                hidden_state_delta: Vec::new(),
                needs_context: Vec::new(),
                next_choices: scene_specific_choices(),
                actor_goal_events: Vec::new(),
                actor_move_events: Vec::new(),
            },
        })?;
        assert_eq!(committed.turn_id, "turn_0001");
        let store_paths = resolve_store_paths(Some(store.as_path()))?;
        let files = world_file_paths(&store_paths, "stw_agent");
        let raw = std::fs::read_to_string(files.dir.join(TURN_COMMITS_FILENAME))?;
        let envelopes = raw
            .lines()
            .map(serde_json::from_str::<TurnCommitEnvelope>)
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(envelopes.len(), 2);
        assert_eq!(envelopes[0].status, TurnCommitStatus::Prepared);
        assert_eq!(envelopes[1].status, TurnCommitStatus::Committed);
        assert_eq!(envelopes[1].parent_turn_id, "turn_0000");
        assert_eq!(envelopes[1].turn_id, "turn_0001");
        assert!(files.dir.join("body_resource_state.json").is_file());
        assert!(files.dir.join("location_graph.json").is_file());
        assert!(files.dir.join("plot_threads.json").is_file());
        assert!(files.dir.join("active_scene_pressures.json").is_file());

        let packet = build_vn_packet(&BuildVnPacketOptions {
            store_root: Some(store),
            world_id: "stw_agent".to_owned(),
            turn_id: Some("turn_0001".to_owned()),
            scene_image_url: None,
        })?;
        assert_eq!(
            packet.scene.text_blocks,
            vec!["agent-authored visible scene"]
        );
        Ok(())
    }

    #[test]
    fn commits_extra_contacts_into_next_pending_context_and_archive() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body("stw_agent_extras"))?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;

        let pending = enqueue_agent_turn(&AgentSubmitTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_agent_extras".to_owned(),
            input: "1".to_owned(),
            narrative_level: None,
        })?;
        commit_agent_turn(&AgentCommitTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_agent_extras".to_owned(),
            response: AgentTurnResponse {
                schema_version: AGENT_TURN_RESPONSE_SCHEMA_VERSION.to_owned(),
                world_id: pending.world_id.clone(),
                turn_id: pending.turn_id.clone(),
                resolution_proposal: None,
                scene_director_proposal: None,
                consequence_proposal: None,
                social_exchange_proposal: None,
                encounter_proposal: None,
                visible_scene: NarrativeScene {
                    schema_version: NARRATIVE_SCENE_SCHEMA_VERSION.to_owned(),
                    speaker: None,
                    text_blocks: vec!["문지기가 피 묻은 소매를 보았다.".to_owned()],
                    tone_notes: Vec::new(),
                },
                adjudication: None,
                canon_event: None,
                entity_updates: Vec::new(),
                relationship_updates: Vec::new(),
                plot_thread_events: Vec::new(),
                scene_pressure_events: Vec::new(),
                world_lore_updates: Vec::new(),
                character_text_design_updates: Vec::new(),
                body_resource_events: Vec::new(),
                location_events: Vec::new(),
                extra_contacts: vec![AgentExtraContact {
                    surface_label: "gate porter".to_owned(),
                    known_name: None,
                    role: Some("porter near the west gate".to_owned()),
                    location_id: Some("place:opening_location".to_owned()),
                    scene_role: Some("witness".to_owned()),
                    contact_summary: "saw the protagonist hide a bloodied sleeve".to_owned(),
                    pressure_tags: vec!["social".to_owned(), "threat".to_owned()],
                    promotion_hint: Some("witnessed risky action".to_owned()),
                    memory_action: Some("remember".to_owned()),
                    disposition: Some("wary".to_owned()),
                    text_design: None,
                    open_hooks: vec!["may report suspicious behavior".to_owned()],
                }],
                hidden_state_delta: Vec::new(),
                needs_context: Vec::new(),
                next_choices: scene_specific_choices(),
                actor_goal_events: Vec::new(),
                actor_move_events: Vec::new(),
            },
        })?;

        let store_paths = resolve_store_paths(Some(store.as_path()))?;
        let files = world_file_paths(&store_paths, "stw_agent_extras");
        let remembered = load_remembered_extras(files.dir.as_path(), "stw_agent_extras")?;
        assert_eq!(remembered.extras.len(), 1);
        assert_eq!(remembered.extras[0].display_name, "gate porter");
        let projection_records = load_extra_memory_projection_records(files.dir.as_path())?;
        assert_eq!(projection_records.len(), 1);
        assert_eq!(
            projection_records[0].status,
            ExtraMemoryProjectionStatus::Committed
        );

        let next_pending = enqueue_agent_turn(&AgentSubmitTurnOptions {
            store_root: Some(store),
            world_id: "stw_agent_extras".to_owned(),
            input: "1".to_owned(),
            narrative_level: None,
        })?;
        assert_eq!(
            next_pending.visible_context.extra_memory.remembered_extras[0].display_name,
            "gate porter"
        );
        Ok(())
    }

    #[test]
    fn rejects_corrupt_extra_memory_before_advancing_turn() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body("stw_agent_corrupt_extras"))?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let store_paths = resolve_store_paths(Some(store.as_path()))?;
        let files = world_file_paths(&store_paths, "stw_agent_corrupt_extras");

        let pending = enqueue_agent_turn(&AgentSubmitTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_agent_corrupt_extras".to_owned(),
            input: "1".to_owned(),
            narrative_level: None,
        })?;
        std::fs::write(files.dir.join(crate::REMEMBERED_EXTRAS_FILENAME), "{")?;
        let result = commit_agent_turn(&AgentCommitTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_agent_corrupt_extras".to_owned(),
            response: AgentTurnResponse {
                schema_version: AGENT_TURN_RESPONSE_SCHEMA_VERSION.to_owned(),
                world_id: pending.world_id.clone(),
                turn_id: pending.turn_id.clone(),
                resolution_proposal: None,
                scene_director_proposal: None,
                consequence_proposal: None,
                social_exchange_proposal: None,
                encounter_proposal: None,
                visible_scene: NarrativeScene {
                    schema_version: NARRATIVE_SCENE_SCHEMA_VERSION.to_owned(),
                    speaker: None,
                    text_blocks: vec!["agent-authored visible scene".to_owned()],
                    tone_notes: Vec::new(),
                },
                adjudication: None,
                canon_event: None,
                entity_updates: Vec::new(),
                relationship_updates: Vec::new(),
                plot_thread_events: Vec::new(),
                scene_pressure_events: Vec::new(),
                world_lore_updates: Vec::new(),
                character_text_design_updates: Vec::new(),
                body_resource_events: Vec::new(),
                location_events: Vec::new(),
                extra_contacts: vec![AgentExtraContact {
                    surface_label: "gate porter".to_owned(),
                    known_name: None,
                    role: Some("porter".to_owned()),
                    location_id: Some("place:opening_location".to_owned()),
                    scene_role: Some("witness".to_owned()),
                    contact_summary: "noticed a torn sleeve".to_owned(),
                    pressure_tags: Vec::new(),
                    promotion_hint: None,
                    memory_action: Some("remember".to_owned()),
                    disposition: None,
                    text_design: None,
                    open_hooks: Vec::new(),
                }],
                hidden_state_delta: Vec::new(),
                needs_context: Vec::new(),
                next_choices: scene_specific_choices(),
                actor_goal_events: Vec::new(),
                actor_move_events: Vec::new(),
            },
        });
        let Err(error) = result else {
            anyhow::bail!("corrupt extra memory should fail before turn advance");
        };

        assert!(format!("{error:#}").contains("remembered_extras.json"));
        let latest: TurnSnapshot = read_json(&files.latest_snapshot)?;
        assert_eq!(latest.turn_id, "turn_0000");
        assert!(load_pending_agent_turn(Some(store.as_path()), "stw_agent_corrupt_extras").is_ok());
        Ok(())
    }

    #[test]
    fn rejects_agent_response_without_complete_next_choices() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body("stw_agent_missing_choices"))?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;

        let pending = enqueue_agent_turn(&AgentSubmitTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_agent_missing_choices".to_owned(),
            input: "1".to_owned(),
            narrative_level: None,
        })?;
        let Err(error) = commit_agent_turn(&AgentCommitTurnOptions {
            store_root: Some(store),
            world_id: "stw_agent_missing_choices".to_owned(),
            response: AgentTurnResponse {
                schema_version: AGENT_TURN_RESPONSE_SCHEMA_VERSION.to_owned(),
                world_id: pending.world_id,
                turn_id: pending.turn_id,
                resolution_proposal: None,
                scene_director_proposal: None,
                consequence_proposal: None,
                social_exchange_proposal: None,
                encounter_proposal: None,
                visible_scene: NarrativeScene {
                    schema_version: NARRATIVE_SCENE_SCHEMA_VERSION.to_owned(),
                    speaker: None,
                    text_blocks: vec!["agent-authored visible scene".to_owned()],
                    tone_notes: Vec::new(),
                },
                adjudication: None,
                canon_event: None,
                entity_updates: Vec::new(),
                relationship_updates: Vec::new(),
                plot_thread_events: Vec::new(),
                scene_pressure_events: Vec::new(),
                world_lore_updates: Vec::new(),
                character_text_design_updates: Vec::new(),
                body_resource_events: Vec::new(),
                location_events: Vec::new(),
                extra_contacts: Vec::new(),
                hidden_state_delta: Vec::new(),
                needs_context: Vec::new(),
                next_choices: Vec::new(),
                actor_goal_events: Vec::new(),
                actor_move_events: Vec::new(),
            },
        }) else {
            anyhow::bail!("empty next_choices reached VN instead of failing");
        };
        assert!(
            error
                .to_string()
                .contains("next_choices must contain exactly slots 1..7")
        );
        Ok(())
    }

    #[test]
    fn rejects_agent_response_that_keeps_default_next_choices() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body("stw_agent_default_choices"))?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;

        let pending = enqueue_agent_turn(&AgentSubmitTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_agent_default_choices".to_owned(),
            input: "1".to_owned(),
            narrative_level: None,
        })?;
        let Err(error) = commit_agent_turn(&AgentCommitTurnOptions {
            store_root: Some(store),
            world_id: "stw_agent_default_choices".to_owned(),
            response: AgentTurnResponse {
                schema_version: AGENT_TURN_RESPONSE_SCHEMA_VERSION.to_owned(),
                world_id: pending.world_id,
                turn_id: pending.turn_id,
                resolution_proposal: None,
                scene_director_proposal: None,
                consequence_proposal: None,
                social_exchange_proposal: None,
                encounter_proposal: None,
                visible_scene: NarrativeScene {
                    schema_version: NARRATIVE_SCENE_SCHEMA_VERSION.to_owned(),
                    speaker: None,
                    text_blocks: vec!["agent-authored visible scene".to_owned()],
                    tone_notes: Vec::new(),
                },
                adjudication: None,
                canon_event: None,
                entity_updates: Vec::new(),
                relationship_updates: Vec::new(),
                plot_thread_events: Vec::new(),
                scene_pressure_events: Vec::new(),
                world_lore_updates: Vec::new(),
                character_text_design_updates: Vec::new(),
                body_resource_events: Vec::new(),
                location_events: Vec::new(),
                extra_contacts: Vec::new(),
                hidden_state_delta: Vec::new(),
                needs_context: Vec::new(),
                next_choices: default_turn_choices(),
                actor_goal_events: Vec::new(),
                actor_move_events: Vec::new(),
            },
        }) else {
            anyhow::bail!("default next_choices survived as agent-authored choices");
        };
        assert!(
            error
                .to_string()
                .contains("default template choices leaked")
        );
        Ok(())
    }

    #[test]
    fn rejects_invalid_resolution_proposal_before_turn_advance() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body("stw_agent_bad_resolution"))?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;

        let pending = enqueue_agent_turn(&AgentSubmitTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_agent_bad_resolution".to_owned(),
            input: "1".to_owned(),
            narrative_level: None,
        })?;
        let Err(error) = commit_agent_turn(&AgentCommitTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_agent_bad_resolution".to_owned(),
            response: AgentTurnResponse {
                schema_version: AGENT_TURN_RESPONSE_SCHEMA_VERSION.to_owned(),
                world_id: pending.world_id.clone(),
                turn_id: pending.turn_id.clone(),
                resolution_proposal: Some(invalid_resolution_proposal(
                    pending.world_id.as_str(),
                    pending.turn_id.as_str(),
                )),
                scene_director_proposal: None,
                consequence_proposal: None,
                social_exchange_proposal: None,
                encounter_proposal: None,
                visible_scene: NarrativeScene {
                    schema_version: NARRATIVE_SCENE_SCHEMA_VERSION.to_owned(),
                    speaker: None,
                    text_blocks: vec!["agent-authored visible scene".to_owned()],
                    tone_notes: Vec::new(),
                },
                adjudication: None,
                canon_event: None,
                entity_updates: Vec::new(),
                relationship_updates: Vec::new(),
                plot_thread_events: Vec::new(),
                scene_pressure_events: Vec::new(),
                world_lore_updates: Vec::new(),
                character_text_design_updates: Vec::new(),
                body_resource_events: Vec::new(),
                location_events: Vec::new(),
                extra_contacts: Vec::new(),
                hidden_state_delta: Vec::new(),
                needs_context: Vec::new(),
                next_choices: scene_specific_choices(),
                actor_goal_events: Vec::new(),
                actor_move_events: Vec::new(),
            },
        }) else {
            anyhow::bail!("invalid resolution proposal reached turn advance");
        };
        assert!(
            format!("{error:#}").contains("resolution proposal audit failed"),
            "{error:#}"
        );

        let store_paths = resolve_store_paths(Some(store.as_path()))?;
        let files = world_file_paths(&store_paths, "stw_agent_bad_resolution");
        let latest: TurnSnapshot = read_json(&files.latest_snapshot)?;
        assert_eq!(latest.turn_id, "turn_0000");
        Ok(())
    }

    #[test]
    fn rejects_agent_choices_that_leak_internal_anchor_ids() {
        let mut response = AgentTurnResponse {
            schema_version: AGENT_TURN_RESPONSE_SCHEMA_VERSION.to_owned(),
            world_id: "stw_anchor_leak".to_owned(),
            turn_id: "turn_0001".to_owned(),
            resolution_proposal: None,
            scene_director_proposal: None,
            consequence_proposal: None,
            social_exchange_proposal: None,
            encounter_proposal: None,
            visible_scene: NarrativeScene {
                schema_version: NARRATIVE_SCENE_SCHEMA_VERSION.to_owned(),
                speaker: None,
                text_blocks: vec!["장면이 이어진다.".to_owned()],
                tone_notes: Vec::new(),
            },
            adjudication: None,
            canon_event: None,
            next_choices: scene_specific_choices(),
            plot_thread_events: Vec::new(),
            scene_pressure_events: Vec::new(),
            entity_updates: Vec::new(),
            relationship_updates: Vec::new(),
            world_lore_updates: Vec::new(),
            character_text_design_updates: Vec::new(),
            body_resource_events: Vec::new(),
            location_events: Vec::new(),
            hidden_state_delta: Vec::new(),
            needs_context: Vec::new(),
            extra_contacts: Vec::new(),
            actor_goal_events: Vec::new(),
            actor_move_events: Vec::new(),
        };
        response.next_choices[2].intent = "char:anchor의 반 걸음 뒤에 멈춰 선다".to_owned();

        let Err(error) = validate_agent_next_choices(&response) else {
            panic!("internal anchor id must reject agent choices");
        };
        assert!(error.to_string().contains("internal token"));
    }

    #[test]
    fn extra_contact_preflight_failure_does_not_advance_turn() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body("stw_agent_extra_preflight"))?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;

        let pending = enqueue_agent_turn(&AgentSubmitTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_agent_extra_preflight".to_owned(),
            input: "1".to_owned(),
            narrative_level: None,
        })?;
        let store_paths = resolve_store_paths(Some(store.as_path()))?;
        let files = world_file_paths(&store_paths, "stw_agent_extra_preflight");
        let before: TurnSnapshot = read_json(&files.latest_snapshot)?;

        let Err(error) = commit_agent_turn(&AgentCommitTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_agent_extra_preflight".to_owned(),
            response: AgentTurnResponse {
                schema_version: AGENT_TURN_RESPONSE_SCHEMA_VERSION.to_owned(),
                world_id: pending.world_id.clone(),
                turn_id: pending.turn_id.clone(),
                resolution_proposal: None,
                scene_director_proposal: None,
                consequence_proposal: None,
                social_exchange_proposal: None,
                encounter_proposal: None,
                visible_scene: NarrativeScene {
                    schema_version: NARRATIVE_SCENE_SCHEMA_VERSION.to_owned(),
                    speaker: None,
                    text_blocks: vec!["문지기가 고개를 들었다.".to_owned()],
                    tone_notes: Vec::new(),
                },
                adjudication: None,
                canon_event: None,
                entity_updates: Vec::new(),
                relationship_updates: Vec::new(),
                plot_thread_events: Vec::new(),
                scene_pressure_events: Vec::new(),
                world_lore_updates: Vec::new(),
                character_text_design_updates: Vec::new(),
                body_resource_events: Vec::new(),
                location_events: Vec::new(),
                extra_contacts: vec![AgentExtraContact {
                    surface_label: "player-visible 주변 인물 표지".to_owned(),
                    known_name: None,
                    role: Some("장소/사건 안 역할".to_owned()),
                    location_id: Some("place:opening_location".to_owned()),
                    scene_role: Some("witness".to_owned()),
                    contact_summary: "이번 턴에서 플레이어-visible로 남은 접촉/목격/거래/감정 흔적"
                        .to_owned(),
                    pressure_tags: Vec::new(),
                    promotion_hint: None,
                    memory_action: Some("trace".to_owned()),
                    disposition: None,
                    text_design: None,
                    open_hooks: Vec::new(),
                }],
                hidden_state_delta: Vec::new(),
                needs_context: Vec::new(),
                next_choices: scene_specific_choices(),
                actor_goal_events: Vec::new(),
                actor_move_events: Vec::new(),
            },
        }) else {
            anyhow::bail!("placeholder extra contact reached world commit");
        };

        assert!(error.to_string().contains("schema placeholder"));
        let after: TurnSnapshot = read_json(&files.latest_snapshot)?;
        assert_eq!(after.turn_id, before.turn_id);
        assert_eq!(after.last_choices.len(), before.last_choices.len());
        assert_eq!(after.last_choices[0].slot, before.last_choices[0].slot);
        assert_eq!(after.last_choices[0].tag, before.last_choices[0].tag);
        let still_pending =
            load_pending_agent_turn(Some(store.as_path()), "stw_agent_extra_preflight")?;
        assert_eq!(still_pending.turn_id, pending.turn_id);
        Ok(())
    }

    #[test]
    fn narrative_budget_levels_are_distinct_and_clamped() {
        let level_one = narrative_budget_for_level(Some(1));
        let level_three = narrative_budget_for_level(Some(3));

        assert_eq!(normalize_narrative_level(None), 1);
        assert_eq!(normalize_narrative_level(Some(0)), 1);
        assert_eq!(normalize_narrative_level(Some(4)), 3);
        assert_ne!(
            level_one.standard_choice_turn,
            level_three.standard_choice_turn
        );
        assert_eq!(level_one.standard_choice_turn_blocks, 6);
        assert_eq!(level_three.standard_choice_turn_blocks, 14);
        assert_eq!(level_three.target_chars, 7_000);
        assert!(level_three.character_budget.contains("8000-12000자"));
    }
}
