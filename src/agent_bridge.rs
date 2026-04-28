use crate::models::{
    AdjudicationGate, CharacterVoiceAnchor, FREEFORM_CHOICE_SLOT, GUIDE_CHOICE_SLOT, HiddenState,
    NARRATIVE_SCENE_SCHEMA_VERSION, NarrativeScene, TurnChoice, TurnSnapshot,
    default_freeform_choice, default_turn_choices, is_guide_choice_tag, normalize_turn_choices,
};
use crate::store::{WorldFilePaths, read_json, resolve_store_paths, world_file_paths, write_json};
use crate::turn::{AdvanceTurnOptions, advance_turn};
use crate::vn::{BuildVnPacketOptions, VnPacket, build_vn_packet};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const AGENT_PENDING_TURN_SCHEMA_VERSION: &str = "singulari.agent_pending_turn.v1";
pub const AGENT_TURN_RESPONSE_SCHEMA_VERSION: &str = "singulari.agent_turn_response.v1";
pub const AGENT_COMMIT_RECORD_SCHEMA_VERSION: &str = "singulari.agent_commit_record.v1";
pub const CODEX_THREAD_BINDING_SCHEMA_VERSION: &str = "singulari.codex_thread_binding.v1";

const AGENT_BRIDGE_DIR: &str = "agent_bridge";
const PENDING_AGENT_TURN_FILENAME: &str = "pending_turn.json";
const COMMITTED_AGENT_TURNS_DIR: &str = "committed_turns";
const AGENT_COMMIT_RECORD_FILENAME: &str = "commit_record.json";
const CODEX_THREAD_BINDING_FILENAME: &str = "codex_thread_binding.json";

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

#[derive(Debug, Clone)]
pub struct SaveCodexThreadBindingOptions {
    pub store_root: Option<PathBuf>,
    pub world_id: String,
    pub thread_id: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexThreadBinding {
    pub schema_version: String,
    pub world_id: String,
    pub thread_id: String,
    pub source: String,
    pub updated_at: String,
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
    pub visible_scene: NarrativeScene,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<AgentResponseAdjudication>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canon_event: Option<AgentResponseCanonEvent>,
    #[serde(default)]
    pub entity_updates: Vec<Value>,
    #[serde(default)]
    pub relationship_updates: Vec<Value>,
    #[serde(default)]
    pub hidden_state_delta: Vec<Value>,
    #[serde(default)]
    pub next_choices: Vec<TurnChoice>,
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
    let pending = PendingAgentTurn {
        schema_version: AGENT_PENDING_TURN_SCHEMA_VERSION.to_owned(),
        world_id: options.world_id.clone(),
        turn_id,
        status: "pending".to_owned(),
        player_input: player_input.to_owned(),
        selected_choice: selected_choice(player_input, &snapshot),
        visible_context: visible_context(&snapshot, &entities, &current_packet),
        private_adjudication_context: private_context(&hidden_state),
        output_contract: AgentOutputContract {
            language: "ko".to_owned(),
            must_return_json: true,
            hidden_truth_must_not_appear_in_visible_text: true,
            narrative_level: normalize_narrative_level(options.narrative_level),
            narrative_budget: narrative_budget_for_level(options.narrative_level),
        },
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

/// Save the Codex thread currently responsible for realtime narrative dispatch.
///
/// # Errors
///
/// Returns an error when the world cannot be resolved, the thread id is empty or
/// control-character-tainted, or the binding file cannot be written.
pub fn save_codex_thread_binding(
    options: &SaveCodexThreadBindingOptions,
) -> Result<CodexThreadBinding> {
    ensure_human_safe_field("thread_id", options.thread_id.as_str())?;
    ensure_human_safe_field("source", options.source.as_str())?;
    let store_paths = resolve_store_paths(options.store_root.as_deref())?;
    let files = world_file_paths(&store_paths, options.world_id.as_str());
    if !files.world.exists() {
        bail!(
            "cannot bind Codex thread for missing world: world_id={}, path={}",
            options.world_id,
            files.world.display()
        );
    }
    let binding = CodexThreadBinding {
        schema_version: CODEX_THREAD_BINDING_SCHEMA_VERSION.to_owned(),
        world_id: options.world_id.clone(),
        thread_id: options.thread_id.trim().to_owned(),
        source: options.source.trim().to_owned(),
        updated_at: Utc::now().to_rfc3339(),
    };
    let binding_path = codex_thread_binding_path(&files);
    ensure_parent_dir(&binding_path)?;
    write_json(&binding_path, &binding)?;
    Ok(binding)
}

/// Load the current Codex thread binding for a world.
///
/// # Errors
///
/// Returns an error when a present binding file is malformed or targets a
/// different world.
pub fn load_codex_thread_binding(
    store_root: Option<&Path>,
    world_id: &str,
) -> Result<Option<CodexThreadBinding>> {
    let store_paths = resolve_store_paths(store_root)?;
    let files = world_file_paths(&store_paths, world_id);
    let binding_path = codex_thread_binding_path(&files);
    if !binding_path.exists() {
        return Ok(None);
    }
    let binding: CodexThreadBinding = read_json(&binding_path)?;
    validate_codex_thread_binding(&binding, world_id)?;
    Ok(Some(binding))
}

/// Remove the current Codex thread binding for a world, if present.
///
/// # Errors
///
/// Returns an error when the binding is malformed or cannot be removed.
pub fn clear_codex_thread_binding(
    store_root: Option<&Path>,
    world_id: &str,
) -> Result<Option<CodexThreadBinding>> {
    let store_paths = resolve_store_paths(store_root)?;
    let files = world_file_paths(&store_paths, world_id);
    let binding_path = codex_thread_binding_path(&files);
    if !binding_path.exists() {
        return Ok(None);
    }
    let binding: CodexThreadBinding = read_json(&binding_path)?;
    validate_codex_thread_binding(&binding, world_id)?;
    fs::remove_file(&binding_path)
        .with_context(|| format!("failed to remove {}", binding_path.display()))?;
    Ok(Some(binding))
}

/// Commit an agent-authored scene and advance the world by the queued input.
///
/// # Errors
///
/// Returns an error when there is no matching pending turn, hidden truth leaks
/// into visible text, or turn persistence fails.
pub fn commit_agent_turn(options: &AgentCommitTurnOptions) -> Result<CommittedAgentTurn> {
    let store_paths = resolve_store_paths(options.store_root.as_deref())?;
    let files = world_file_paths(&store_paths, options.world_id.as_str());
    let pending_path = pending_agent_turn_path(&files);
    let pending =
        load_pending_agent_turn(options.store_root.as_deref(), options.world_id.as_str())?;
    let response = canonical_agent_turn_response(options.response.clone());
    validate_agent_response(&pending, &response)?;

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
    if choices_keep_default_template(&response.next_choices) {
        bail!(
            "agent response next_choices must be scene-specific; default template choices leaked"
        );
    }
    Ok(())
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

fn visible_context(
    snapshot: &TurnSnapshot,
    entities: &crate::models::EntityRecords,
    current_packet: &VnPacket,
) -> AgentVisibleContext {
    AgentVisibleContext {
        location: snapshot.protagonist_state.location.clone(),
        recent_scene: current_packet.scene.text_blocks.clone(),
        known_facts: known_facts(snapshot),
        voice_anchors: entities
            .characters
            .iter()
            .filter(|character| !character.voice_anchor.is_empty())
            .map(|character| AgentVoiceAnchor {
                character_id: character.id.clone(),
                name: character.name.visible.clone(),
                anchor: character.voice_anchor.clone(),
            })
            .collect(),
    }
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

fn codex_thread_binding_path(files: &WorldFilePaths) -> PathBuf {
    files
        .dir
        .join(AGENT_BRIDGE_DIR)
        .join(CODEX_THREAD_BINDING_FILENAME)
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

fn validate_codex_thread_binding(
    binding: &CodexThreadBinding,
    expected_world_id: &str,
) -> Result<()> {
    if binding.schema_version != CODEX_THREAD_BINDING_SCHEMA_VERSION {
        bail!(
            "Codex thread binding schema_version mismatch: expected {}, got {}",
            CODEX_THREAD_BINDING_SCHEMA_VERSION,
            binding.schema_version
        );
    }
    if binding.world_id != expected_world_id {
        bail!(
            "Codex thread binding world mismatch: expected {}, got {}",
            expected_world_id,
            binding.world_id
        );
    }
    ensure_human_safe_field("thread_id", binding.thread_id.as_str())?;
    ensure_human_safe_field("source", binding.source.as_str())?;
    Ok(())
}

fn ensure_human_safe_field(field: &str, value: &str) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("Codex thread binding {field} must not be empty");
    }
    if trimmed.chars().any(char::is_control) {
        bail!("Codex thread binding {field} must not contain control characters");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        AGENT_TURN_RESPONSE_SCHEMA_VERSION, AgentCommitTurnOptions, AgentSubmitTurnOptions,
        AgentTurnResponse, SaveCodexThreadBindingOptions, canonical_agent_turn_response,
        clear_codex_thread_binding, enqueue_agent_turn, load_codex_thread_binding,
        narrative_budget_for_level, normalize_narrative_level, save_codex_thread_binding,
        selected_choice,
    };
    use crate::agent_bridge::commit_agent_turn;
    use crate::models::{
        GUIDE_CHOICE_TAG, NARRATIVE_SCENE_SCHEMA_VERSION, NarrativeScene, TurnChoice, TurnSnapshot,
        default_turn_choices,
    };
    use crate::store::{
        InitWorldOptions, init_world, read_json, resolve_store_paths, world_file_paths,
    };
    use crate::vn::{BuildVnPacketOptions, build_vn_packet};
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
                tag: "발소리".to_owned(),
                intent: "젖은 흙 위에 새로 찍힌 발자국을 따라 조심스럽게 움직인다".to_owned(),
            },
            TurnChoice {
                slot: 2,
                tag: "몸 상태".to_owned(),
                intent: "손목의 통증과 낯선 장비가 지금 가능한 행동을 얼마나 제한하는지 살핀다"
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
                intent: "방금 본 이끼 낀 문장과 발자국의 의미를 세계 기록에서 대조한다".to_owned(),
            },
            TurnChoice {
                slot: 5,
                tag: "먼 시야".to_owned(),
                intent: "이 장소를 둘러싼 숲길과 사람들의 이동 흐름을 한 박자 멀리서 본다"
                    .to_owned(),
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
            hidden_state_delta: Vec::new(),
            next_choices: legacy_slot_contract_choices(),
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
                hidden_state_delta: Vec::new(),
                next_choices: scene_specific_choices(),
            },
        })?;
        assert_eq!(committed.turn_id, "turn_0001");

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
                hidden_state_delta: Vec::new(),
                next_choices: Vec::new(),
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
                hidden_state_delta: Vec::new(),
                next_choices: default_turn_choices(),
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
    fn codex_thread_binding_round_trips_and_clears() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body("stw_codex_bind"))?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;

        let binding = save_codex_thread_binding(&SaveCodexThreadBindingOptions {
            store_root: Some(store.clone()),
            world_id: "stw_codex_bind".to_owned(),
            thread_id: "codex-thread-test-001".to_owned(),
            source: "test".to_owned(),
        })?;
        assert_eq!(binding.world_id, "stw_codex_bind");
        assert_eq!(binding.source, "test");

        let Some(loaded) = load_codex_thread_binding(Some(store.as_path()), "stw_codex_bind")?
        else {
            anyhow::bail!("binding should be present after save");
        };
        assert_eq!(loaded.thread_id, binding.thread_id);

        let Some(cleared) = clear_codex_thread_binding(Some(store.as_path()), "stw_codex_bind")?
        else {
            anyhow::bail!("binding should be returned when cleared");
        };
        assert_eq!(cleared.thread_id, binding.thread_id);
        assert!(load_codex_thread_binding(Some(store.as_path()), "stw_codex_bind")?.is_none());
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
