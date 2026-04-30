use crate::models::TurnChoice;
use crate::store::{append_jsonl, read_json, write_json};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

pub const HOOK_LEDGER_PACKET_SCHEMA_VERSION: &str = "singulari.hook_ledger_packet.v1";
pub const HOOK_LEDGER_STATE_SCHEMA_VERSION: &str = "singulari.hook_ledger_state.v1";
pub const HOOK_EVENT_SCHEMA_VERSION: &str = "singulari.hook_event.v1";
pub const OFFERED_CHOICE_SET_SCHEMA_VERSION: &str = "singulari.offered_choice_set.v1";
pub const UNCHOSEN_ECHO_SCHEMA_VERSION: &str = "singulari.unchosen_echo.v1";
pub const ECHO_LEDGER_STATE_SCHEMA_VERSION: &str = "singulari.echo_ledger_state.v1";
pub const SESSION_RECEIPT_SCHEMA_VERSION: &str = "singulari.session_receipt.v1";

pub const HOOK_THREADS_FILENAME: &str = "hook_threads.json";
pub const HOOK_EVENTS_FILENAME: &str = "hook_events.jsonl";
pub const OFFERED_CHOICE_SETS_FILENAME: &str = "offered_choice_sets.jsonl";
pub const UNCHOSEN_ECHOES_FILENAME: &str = "unchosen_echoes.json";
pub const SESSION_RECEIPT_FILENAME: &str = "session_receipt.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookPacket {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub active_promises: Vec<HookThread>,
    #[serde(default)]
    pub due_promises: Vec<HookThread>,
    #[serde(default)]
    pub active_echoes: Vec<UnchosenEcho>,
    #[serde(default)]
    pub returning_echoes: Vec<UnchosenEcho>,
    #[serde(default)]
    pub choice_biases: Vec<HookChoiceBias>,
    pub tea_recap: TeaRecap,
    pub compiler_policy: HookLedgerPolicy,
}

impl Default for HookPacket {
    fn default() -> Self {
        Self {
            schema_version: HOOK_LEDGER_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            turn_id: String::new(),
            active_promises: Vec::new(),
            due_promises: Vec::new(),
            active_echoes: Vec::new(),
            returning_echoes: Vec::new(),
            choice_biases: Vec::new(),
            tea_recap: TeaRecap::default(),
            compiler_policy: HookLedgerPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookLedgerState {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub active_threads: Vec<HookThread>,
    pub compiler_policy: HookLedgerPolicy,
}

impl HookLedgerState {
    #[must_use]
    pub fn empty(world_id: &str, turn_id: &str) -> Self {
        Self {
            schema_version: HOOK_LEDGER_STATE_SCHEMA_VERSION.to_owned(),
            world_id: world_id.to_owned(),
            turn_id: turn_id.to_owned(),
            active_threads: Vec::new(),
            compiler_policy: HookLedgerPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EchoLedgerState {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub active_echoes: Vec<UnchosenEcho>,
    pub compiler_policy: HookLedgerPolicy,
}

impl EchoLedgerState {
    #[must_use]
    pub fn empty(world_id: &str, turn_id: &str) -> Self {
        Self {
            schema_version: ECHO_LEDGER_STATE_SCHEMA_VERSION.to_owned(),
            world_id: world_id.to_owned(),
            turn_id: turn_id.to_owned(),
            active_echoes: Vec::new(),
            compiler_policy: HookLedgerPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookLedgerPolicy {
    pub source: String,
    pub max_new_promises_per_turn: u8,
    pub max_new_echoes_per_turn: u8,
    pub max_teases_without_progress: u8,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for HookLedgerPolicy {
    fn default() -> Self {
        Self {
            source: "promise_ledger_unchosen_echoes_mvp".to_owned(),
            max_new_promises_per_turn: 1,
            max_new_echoes_per_turn: 1,
            max_teases_without_progress: 3,
            use_rules: vec![
                "HookPacket is a player-visible advisory packet, not hidden truth.".to_owned(),
                "Promises must be evidence-backed and payoff-bound.".to_owned(),
                "Unchosen Echoes must come from choices actually shown to the player.".to_owned(),
                "Echoes are non-punitive and may not encode real-time FOMO.".to_owned(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookThread {
    pub hook_id: String,
    pub kind: HookKind,
    pub visible_promise: String,
    #[serde(default)]
    pub anchor_refs: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub opened_by_event: String,
    pub payoff_contract: PayoffContract,
    pub return_rights: HookReturnRights,
    pub fatigue_score: u8,
    pub status: HookStatus,
    pub opened_turn_id: String,
    pub last_touched_turn_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookKind {
    Mystery,
    RelationshipDebt,
    PlaceMemory,
    BodyScar,
    ProcessCountdown,
    Rumor,
    PersonalAnchor,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookStatus {
    Opened,
    Progressing,
    PayoffDue,
    PaidOff,
    Suppressed,
    Archived,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PayoffContract {
    #[serde(default)]
    pub owed_payoff: Vec<PayoffKind>,
    pub minimum_progress_interval: u32,
    pub max_teases_without_progress: u8,
    #[serde(default)]
    pub possible_resolutions: Vec<ResolutionMode>,
}

impl Default for PayoffContract {
    fn default() -> Self {
        Self {
            owed_payoff: vec![PayoffKind::Progress],
            minimum_progress_interval: 3,
            max_teases_without_progress: 3,
            possible_resolutions: vec![ResolutionMode::Clarified, ResolutionMode::Transformed],
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PayoffKind {
    Progress,
    Reveal,
    RelationshipShift,
    Cost,
    AccessChange,
    Reframe,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionMode {
    Clarified,
    Misread,
    PaidCost,
    RelationshipChanged,
    Transformed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookReturnRights {
    #[serde(default)]
    pub touch_conditions: Vec<TouchCondition>,
    pub may_bias_choice: bool,
    pub may_enter_recap: bool,
}

impl Default for HookReturnRights {
    fn default() -> Self {
        Self {
            touch_conditions: vec![TouchCondition::SceneContact],
            may_bias_choice: true,
            may_enter_recap: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TouchCondition {
    SceneContact,
    SameLocation,
    SameActor,
    ProcessTick,
    SessionEnd,
    PlayerAsks,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OmissionProfile {
    pub can_create_echo: bool,
    pub echo_weight: u8,
    pub omission_meaning: OmissionMeaning,
    #[serde(default)]
    pub visible_stakes: Vec<String>,
    #[serde(default)]
    pub return_conditions: Vec<TouchCondition>,
    pub non_punitive: bool,
}

impl Default for OmissionProfile {
    fn default() -> Self {
        Self {
            can_create_echo: false,
            echo_weight: 0,
            omission_meaning: OmissionMeaning::IgnoredSurface,
            visible_stakes: Vec::new(),
            return_conditions: vec![TouchCondition::SceneContact],
            non_punitive: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OmissionMeaning {
    UnaskedQuestion,
    IgnoredSurface,
    DeferredCare,
    UnchallengedLie,
    UnusedLeverage,
    UnansweredAppeal,
    UnspokenName,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OfferedChoiceSet {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub scene_id: String,
    #[serde(default)]
    pub choices: Vec<OfferedChoice>,
    #[serde(default)]
    pub visible_context_refs: Vec<String>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OfferedChoice {
    pub choice_id: String,
    pub slot: u8,
    pub visible_label: String,
    pub visible_intent: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub omission_profile: OmissionProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnchosenEcho {
    pub schema_version: String,
    pub echo_id: String,
    pub source_turn_id: String,
    pub unchosen_choice_id: String,
    pub visible_summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implied_meaning: Option<String>,
    #[serde(default)]
    pub anchor_refs: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub return_conditions: Vec<TouchCondition>,
    #[serde(default)]
    pub possible_payoffs: Vec<PayoffKind>,
    pub decay: DecayMode,
    pub status: EchoStatus,
    pub created_turn_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DecayMode {
    SoftenAfterScene,
    ArchiveAfterPayoff,
    LongAnchor,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EchoStatus {
    Active,
    Returning,
    PaidOff,
    Softened,
    Archived,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookChoiceBias {
    pub source_ref: String,
    pub target_ref: String,
    pub bias: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TeaRecap {
    pub schema_version: String,
    #[serde(default)]
    pub remaining_fragrance: Vec<String>,
    #[serde(default)]
    pub remaining_words: Vec<String>,
    #[serde(default)]
    pub residual_heat: Vec<String>,
}

impl Default for TeaRecap {
    fn default() -> Self {
        Self {
            schema_version: SESSION_RECEIPT_SCHEMA_VERSION.to_owned(),
            remaining_fragrance: Vec::new(),
            remaining_words: Vec::new(),
            residual_heat: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionReceipt {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub tea_recap: TeaRecap,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentHookEvent {
    #[serde(default = "default_hook_event_schema")]
    pub schema_version: String,
    pub event_kind: HookEventKind,
    pub hook_id: String,
    pub kind: HookKind,
    pub visible_promise: String,
    #[serde(default)]
    pub anchor_refs: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub opened_by_event: String,
    #[serde(default)]
    pub payoff_contract: PayoffContract,
    #[serde(default)]
    pub return_rights: HookReturnRights,
    #[serde(default)]
    pub summary: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookEventKind {
    Opened,
    Progressed,
    PayoffDue,
    PaidOff,
    Suppressed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookEventRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub event_kind: HookEventKind,
    pub hook_id: String,
    pub kind: HookKind,
    pub visible_promise: String,
    #[serde(default)]
    pub anchor_refs: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub opened_by_event: String,
    pub payoff_contract: PayoffContract,
    pub return_rights: HookReturnRights,
    pub recorded_at: String,
}

#[must_use]
pub fn default_hook_event_schema() -> String {
    HOOK_EVENT_SCHEMA_VERSION.to_owned()
}

/// Creates the per-world hook ledger sidecar files.
///
/// # Errors
///
/// Returns an error when any ledger sidecar cannot be written.
pub fn initialize_hook_ledger_files(world_dir: &Path, world_id: &str, turn_id: &str) -> Result<()> {
    write_json(
        &world_dir.join(HOOK_THREADS_FILENAME),
        &HookLedgerState::empty(world_id, turn_id),
    )?;
    write_json(
        &world_dir.join(UNCHOSEN_ECHOES_FILENAME),
        &EchoLedgerState::empty(world_id, turn_id),
    )?;
    write_json(
        &world_dir.join(SESSION_RECEIPT_FILENAME),
        &SessionReceipt {
            schema_version: SESSION_RECEIPT_SCHEMA_VERSION.to_owned(),
            world_id: world_id.to_owned(),
            turn_id: turn_id.to_owned(),
            tea_recap: TeaRecap::default(),
            updated_at: Utc::now().to_rfc3339(),
        },
    )?;
    fs::write(world_dir.join(HOOK_EVENTS_FILENAME), "").with_context(|| {
        format!(
            "failed to write {}",
            world_dir.join(HOOK_EVENTS_FILENAME).display()
        )
    })?;
    fs::write(world_dir.join(OFFERED_CHOICE_SETS_FILENAME), "").with_context(|| {
        format!(
            "failed to write {}",
            world_dir.join(OFFERED_CHOICE_SETS_FILENAME).display()
        )
    })?;
    Ok(())
}

/// Loads the current player-visible hook packet for prompt/VN surfaces.
///
/// # Errors
///
/// Returns an error when the loaded ledger state cannot be converted into a
/// hook packet.
pub fn load_hook_packet_state(
    world_dir: &Path,
    world_id: &str,
    turn_id: &str,
) -> Result<HookPacket> {
    let hook_state = read_json::<HookLedgerState>(&world_dir.join(HOOK_THREADS_FILENAME))
        .unwrap_or_else(|_| HookLedgerState::empty(world_id, turn_id));
    let echo_state = read_json::<EchoLedgerState>(&world_dir.join(UNCHOSEN_ECHOES_FILENAME))
        .unwrap_or_else(|_| EchoLedgerState::empty(world_id, turn_id));
    Ok(hook_packet_from_states(
        world_id,
        turn_id,
        &hook_state,
        &echo_state,
    ))
}

/// Appends accepted agent-authored promise events and rebuilds the packet.
///
/// # Errors
///
/// Returns an error when the event payload is invalid or ledger files cannot
/// be appended/rebuilt.
pub fn append_agent_hook_events(
    world_dir: &Path,
    world_id: &str,
    turn_id: &str,
    events: &[AgentHookEvent],
) -> Result<HookPacket> {
    let accepted = accepted_agent_hook_events(events)?;
    for event in &accepted {
        append_jsonl(
            &world_dir.join(HOOK_EVENTS_FILENAME),
            &HookEventRecord {
                schema_version: HOOK_EVENT_SCHEMA_VERSION.to_owned(),
                world_id: world_id.to_owned(),
                turn_id: turn_id.to_owned(),
                event_kind: event.event_kind,
                hook_id: event.hook_id.clone(),
                kind: event.kind,
                visible_promise: event.visible_promise.clone(),
                anchor_refs: event.anchor_refs.clone(),
                evidence_refs: event.evidence_refs.clone(),
                opened_by_event: event.opened_by_event.clone(),
                payoff_contract: event.payoff_contract.clone(),
                return_rights: event.return_rights.clone(),
                recorded_at: Utc::now().to_rfc3339(),
            },
        )?;
    }
    rebuild_hook_packet(world_dir, world_id, turn_id)
}

/// Records the offered choice set and materializes at most one unchosen echo.
///
/// # Errors
///
/// Returns an error when choice-set or echo sidecars cannot be read or written.
pub fn record_offered_choice_set_and_echo(
    world_dir: &Path,
    world_id: &str,
    turn_id: &str,
    scene_id: &str,
    choices: &[TurnChoice],
    selected_slot: Option<u8>,
) -> Result<HookPacket> {
    let offered = offered_choice_set(world_id, turn_id, scene_id, choices);
    append_jsonl(&world_dir.join(OFFERED_CHOICE_SETS_FILENAME), &offered)?;
    if let Some(echo) = create_unchosen_echo(&offered, selected_slot) {
        let mut echo_state =
            read_json::<EchoLedgerState>(&world_dir.join(UNCHOSEN_ECHOES_FILENAME))
                .unwrap_or_else(|_| EchoLedgerState::empty(world_id, turn_id));
        if !echo_state
            .active_echoes
            .iter()
            .any(|existing| existing.echo_id == echo.echo_id)
        {
            echo_state.active_echoes.push(echo);
        }
        world_id.clone_into(&mut echo_state.world_id);
        turn_id.clone_into(&mut echo_state.turn_id);
        write_json(&world_dir.join(UNCHOSEN_ECHOES_FILENAME), &echo_state)?;
    }
    rebuild_hook_packet(world_dir, world_id, turn_id)
}

/// Rebuilds hook threads and the session receipt from append-only event state.
///
/// # Errors
///
/// Returns an error when hook event records cannot be loaded or rebuilt state
/// cannot be written.
pub fn rebuild_hook_packet(world_dir: &Path, world_id: &str, turn_id: &str) -> Result<HookPacket> {
    let records = load_hook_event_records(&world_dir.join(HOOK_EVENTS_FILENAME))?;
    let mut threads = Vec::<HookThread>::new();
    for record in records {
        apply_hook_event_record(&mut threads, record);
    }
    let hook_state = HookLedgerState {
        schema_version: HOOK_LEDGER_STATE_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        active_threads: threads,
        compiler_policy: HookLedgerPolicy::default(),
    };
    write_json(&world_dir.join(HOOK_THREADS_FILENAME), &hook_state)?;
    let mut echo_state = read_json::<EchoLedgerState>(&world_dir.join(UNCHOSEN_ECHOES_FILENAME))
        .unwrap_or_else(|_| EchoLedgerState::empty(world_id, turn_id));
    world_id.clone_into(&mut echo_state.world_id);
    turn_id.clone_into(&mut echo_state.turn_id);
    write_json(&world_dir.join(UNCHOSEN_ECHOES_FILENAME), &echo_state)?;
    let packet = hook_packet_from_states(world_id, turn_id, &hook_state, &echo_state);
    write_json(
        &world_dir.join(SESSION_RECEIPT_FILENAME),
        &SessionReceipt {
            schema_version: SESSION_RECEIPT_SCHEMA_VERSION.to_owned(),
            world_id: world_id.to_owned(),
            turn_id: turn_id.to_owned(),
            tea_recap: packet.tea_recap.clone(),
            updated_at: Utc::now().to_rfc3339(),
        },
    )?;
    Ok(packet)
}

/// Validates agent hook events before they enter the authoritative ledger.
///
/// # Errors
///
/// Returns an error when an event is missing its id, visible promise,
/// evidence, opener, or payoff contract, or when too many promise events are
/// submitted for one turn.
pub fn accepted_agent_hook_events(events: &[AgentHookEvent]) -> Result<Vec<AgentHookEvent>> {
    if events.len() > usize::from(HookLedgerPolicy::default().max_new_promises_per_turn) {
        bail!(
            "hook ledger accepts at most one new promise event per turn: actual={}",
            events.len()
        );
    }
    let mut accepted = Vec::new();
    for event in events {
        if event.hook_id.trim().is_empty() {
            bail!("hook event missing hook_id");
        }
        if event.visible_promise.trim().is_empty() {
            bail!(
                "hook event missing visible_promise: hook_id={}",
                event.hook_id
            );
        }
        if event.evidence_refs.is_empty() {
            bail!(
                "hook event missing evidence_refs: hook_id={}, visible_promise={}",
                event.hook_id,
                event.visible_promise
            );
        }
        if event.opened_by_event.trim().is_empty() {
            bail!(
                "hook event missing opened_by_event: hook_id={}",
                event.hook_id
            );
        }
        if event.payoff_contract.owed_payoff.is_empty()
            || event.payoff_contract.possible_resolutions.is_empty()
        {
            bail!(
                "hook event missing payoff contract: hook_id={}",
                event.hook_id
            );
        }
        accepted.push(event.clone());
    }
    Ok(accepted)
}

#[must_use]
pub fn omission_profile_for_choice(
    slot: u8,
    label: &str,
    intent: &str,
    risk_tags: &[String],
    evidence_refs: &[String],
) -> OmissionProfile {
    if !(1..=5).contains(&slot) || evidence_refs.is_empty() {
        return OmissionProfile::default();
    }
    let text = format!("{label} {intent}").to_lowercase();
    let omission_meaning = omission_meaning_from_text(text.as_str(), risk_tags);
    let mut visible_stakes = Vec::new();
    if text.contains("묻") || text.contains("말") || text.contains("이름") {
        visible_stakes.push("말하지 않은 질문이 다음 접촉의 거리감을 바꿀 수 있다.".to_owned());
    } else if text.contains("살피") || text.contains("조사") || text.contains("문") {
        visible_stakes.push("살피지 않은 표면이 장면의 미결로 남을 수 있다.".to_owned());
    } else if !risk_tags.is_empty() {
        visible_stakes.push("이 선택지는 장면 압력과 연결되어 있다.".to_owned());
    } else {
        visible_stakes.push("선택하지 않은 행동의 결이 약하게 남을 수 있다.".to_owned());
    }
    let echo_weight = if risk_tags.is_empty() { 4 } else { 7 };
    OmissionProfile {
        can_create_echo: true,
        echo_weight,
        omission_meaning,
        visible_stakes,
        return_conditions: vec![TouchCondition::SceneContact],
        non_punitive: true,
    }
}

fn offered_choice_set(
    world_id: &str,
    turn_id: &str,
    scene_id: &str,
    choices: &[TurnChoice],
) -> OfferedChoiceSet {
    let choices = choices
        .iter()
        .filter(|choice| (1..=5).contains(&choice.slot))
        .map(|choice| {
            let evidence_refs = vec![
                format!("turn:{turn_id}:choice:{}", choice.slot),
                format!("choice:{}", choice.slot),
            ];
            OfferedChoice {
                choice_id: format!("choice:{}", choice.slot),
                slot: choice.slot,
                visible_label: choice.tag.clone(),
                visible_intent: choice.player_visible_intent().to_owned(),
                omission_profile: omission_profile_for_choice(
                    choice.slot,
                    choice.tag.as_str(),
                    choice.player_visible_intent(),
                    &[],
                    &evidence_refs,
                ),
                evidence_refs,
            }
        })
        .collect();
    OfferedChoiceSet {
        schema_version: OFFERED_CHOICE_SET_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        scene_id: scene_id.to_owned(),
        choices,
        visible_context_refs: vec!["visible_state.choices".to_owned()],
        recorded_at: Utc::now().to_rfc3339(),
    }
}

fn create_unchosen_echo(
    offered: &OfferedChoiceSet,
    selected_slot: Option<u8>,
) -> Option<UnchosenEcho> {
    offered
        .choices
        .iter()
        .filter(|choice| Some(choice.slot) != selected_slot)
        .filter(|choice| choice.omission_profile.can_create_echo)
        .filter(|choice| choice.omission_profile.non_punitive)
        .filter(|choice| !choice.omission_profile.visible_stakes.is_empty())
        .max_by_key(|choice| choice.omission_profile.echo_weight)
        .map(|choice| UnchosenEcho {
            schema_version: UNCHOSEN_ECHO_SCHEMA_VERSION.to_owned(),
            echo_id: format!("echo:{}:slot_{}", offered.turn_id, choice.slot),
            source_turn_id: offered.turn_id.clone(),
            unchosen_choice_id: choice.choice_id.clone(),
            visible_summary: echo_visible_summary(choice),
            implied_meaning: Some(format!("{:?}", choice.omission_profile.omission_meaning)),
            anchor_refs: vec![offered.scene_id.clone()],
            evidence_refs: choice.evidence_refs.clone(),
            return_conditions: choice.omission_profile.return_conditions.clone(),
            possible_payoffs: vec![PayoffKind::Progress, PayoffKind::Reframe],
            decay: DecayMode::SoftenAfterScene,
            status: EchoStatus::Active,
            created_turn_id: offered.turn_id.clone(),
        })
}

fn echo_visible_summary(choice: &OfferedChoice) -> String {
    match choice.omission_profile.omission_meaning {
        OmissionMeaning::UnaskedQuestion => {
            format!(
                "'{}'라는 질문은 그 자리에서 입 밖으로 나오지 않았다.",
                choice.visible_label
            )
        }
        OmissionMeaning::UnspokenName => {
            format!(
                "'{}'에 닿는 이름은 아직 말해지지 않았다.",
                choice.visible_label
            )
        }
        OmissionMeaning::DeferredCare => {
            format!(
                "'{}'로 돌볼 수 있던 일이 잠시 뒤로 밀렸다.",
                choice.visible_label
            )
        }
        OmissionMeaning::UnchallengedLie => {
            format!(
                "'{}'의 모순은 그 순간 바로 찔리지 않았다.",
                choice.visible_label
            )
        }
        OmissionMeaning::UnusedLeverage => {
            format!(
                "'{}'로 쓸 수 있던 단서는 아직 쓰이지 않았다.",
                choice.visible_label
            )
        }
        OmissionMeaning::UnansweredAppeal => {
            format!("'{}'에 담긴 요청은 대답 없이 남았다.", choice.visible_label)
        }
        OmissionMeaning::IgnoredSurface => {
            format!(
                "'{}'의 표면은 지나쳤지만, 장면에는 결이 남았다.",
                choice.visible_label
            )
        }
    }
}

fn omission_meaning_from_text(text: &str, risk_tags: &[String]) -> OmissionMeaning {
    if text.contains("묻") || text.contains("질문") {
        OmissionMeaning::UnaskedQuestion
    } else if text.contains("이름") || text.contains("정체") {
        OmissionMeaning::UnspokenName
    } else if text.contains("돌보") || text.contains("치료") || text.contains("쉰") {
        OmissionMeaning::DeferredCare
    } else if text.contains("거짓") || text.contains("모순") {
        OmissionMeaning::UnchallengedLie
    } else if text.contains("단서") || text.contains("증거") || text.contains("이용") {
        OmissionMeaning::UnusedLeverage
    } else if text.contains("요청")
        || text.contains("도와")
        || text.contains("시선")
        || risk_tags.iter().any(|tag| tag.contains("social"))
    {
        OmissionMeaning::UnansweredAppeal
    } else {
        OmissionMeaning::IgnoredSurface
    }
}

fn load_hook_event_records(path: &Path) -> Result<Vec<HookEventRecord>> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut records = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        records.push(
            serde_json::from_str::<HookEventRecord>(line).with_context(|| {
                format!("failed to parse {} line {}", path.display(), index + 1)
            })?,
        );
    }
    Ok(records)
}

fn apply_hook_event_record(threads: &mut Vec<HookThread>, record: HookEventRecord) {
    let existing = threads
        .iter_mut()
        .find(|thread| thread.hook_id == record.hook_id);
    match record.event_kind {
        HookEventKind::Opened => {
            if existing.is_none() {
                threads.push(HookThread {
                    hook_id: record.hook_id,
                    kind: record.kind,
                    visible_promise: record.visible_promise,
                    anchor_refs: record.anchor_refs,
                    evidence_refs: record.evidence_refs,
                    opened_by_event: record.opened_by_event,
                    payoff_contract: record.payoff_contract,
                    return_rights: record.return_rights,
                    fatigue_score: 0,
                    status: HookStatus::Opened,
                    opened_turn_id: record.turn_id.clone(),
                    last_touched_turn_id: record.turn_id,
                });
            }
        }
        HookEventKind::Progressed | HookEventKind::PayoffDue | HookEventKind::Suppressed => {
            if let Some(thread) = existing {
                thread.status = match record.event_kind {
                    HookEventKind::Progressed => HookStatus::Progressing,
                    HookEventKind::PayoffDue => HookStatus::PayoffDue,
                    HookEventKind::Suppressed => HookStatus::Suppressed,
                    HookEventKind::Opened | HookEventKind::PaidOff => thread.status,
                };
                thread.last_touched_turn_id = record.turn_id;
                thread.evidence_refs = dedupe_refs(
                    thread
                        .evidence_refs
                        .iter()
                        .chain(record.evidence_refs.iter())
                        .cloned(),
                );
            }
        }
        HookEventKind::PaidOff => {
            if let Some(thread) = existing {
                thread.status = HookStatus::PaidOff;
                thread.last_touched_turn_id = record.turn_id;
            }
        }
    }
}

fn hook_packet_from_states(
    world_id: &str,
    turn_id: &str,
    hook_state: &HookLedgerState,
    echo_state: &EchoLedgerState,
) -> HookPacket {
    let active_promises = hook_state
        .active_threads
        .iter()
        .filter(|thread| {
            matches!(
                thread.status,
                HookStatus::Opened | HookStatus::Progressing | HookStatus::PayoffDue
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    let due_promises = active_promises
        .iter()
        .filter(|thread| thread.status == HookStatus::PayoffDue)
        .cloned()
        .collect::<Vec<_>>();
    let active_echoes = echo_state
        .active_echoes
        .iter()
        .filter(|echo| matches!(echo.status, EchoStatus::Active | EchoStatus::Returning))
        .cloned()
        .collect::<Vec<_>>();
    let returning_echoes = active_echoes.iter().take(1).cloned().collect::<Vec<_>>();
    let mut choice_biases = due_promises
        .iter()
        .map(|thread| HookChoiceBias {
            source_ref: thread.hook_id.clone(),
            target_ref: thread
                .anchor_refs
                .first()
                .cloned()
                .unwrap_or_else(|| "current_scene".to_owned()),
            bias: "progress_or_payoff_due_promise".to_owned(),
            evidence_refs: thread.evidence_refs.clone(),
        })
        .collect::<Vec<_>>();
    choice_biases.extend(returning_echoes.iter().map(|echo| {
        HookChoiceBias {
            source_ref: echo.echo_id.clone(),
            target_ref: echo
                .anchor_refs
                .first()
                .cloned()
                .unwrap_or_else(|| "current_scene".to_owned()),
            bias: "return_non_punitive_unchosen_echo".to_owned(),
            evidence_refs: echo.evidence_refs.clone(),
        }
    }));
    HookPacket {
        schema_version: HOOK_LEDGER_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        tea_recap: tea_recap(&active_promises, &active_echoes),
        active_promises,
        due_promises,
        active_echoes,
        returning_echoes,
        choice_biases,
        compiler_policy: HookLedgerPolicy::default(),
    }
}

fn tea_recap(promises: &[HookThread], echoes: &[UnchosenEcho]) -> TeaRecap {
    TeaRecap {
        schema_version: SESSION_RECEIPT_SCHEMA_VERSION.to_owned(),
        remaining_fragrance: promises
            .iter()
            .take(3)
            .map(|thread| thread.visible_promise.clone())
            .collect(),
        remaining_words: echoes
            .iter()
            .take(3)
            .map(|echo| echo.visible_summary.clone())
            .collect(),
        residual_heat: promises
            .iter()
            .filter(|thread| thread.status == HookStatus::PayoffDue)
            .take(2)
            .map(|thread| {
                format!(
                    "{}: 다음 접촉에서 진전 또는 상환이 필요하다.",
                    thread.hook_id
                )
            })
            .collect(),
    }
}

fn dedupe_refs(values: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for value in values {
        if seen.insert(value.clone()) {
            out.push(value);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn choices() -> Vec<TurnChoice> {
        vec![
            TurnChoice {
                slot: 1,
                tag: "이름을 묻는다".to_owned(),
                intent: "문지기가 반응한 이름을 바로 확인한다".to_owned(),
            },
            TurnChoice {
                slot: 2,
                tag: "걸쇠를 살핀다".to_owned(),
                intent: "안쪽에서 휘어진 자국을 확인한다".to_owned(),
            },
            TurnChoice {
                slot: 6,
                tag: "자유서술".to_owned(),
                intent: "6 뒤에 행동을 쓴다".to_owned(),
            },
        ]
    }

    #[test]
    fn records_at_most_one_unchosen_echo_from_displayed_slots() -> anyhow::Result<()> {
        let temp = tempdir()?;
        initialize_hook_ledger_files(temp.path(), "stw_hook", "turn_0000")?;

        let packet = record_offered_choice_set_and_echo(
            temp.path(),
            "stw_hook",
            "turn_0001",
            "place:north_gate",
            &choices(),
            Some(2),
        )?;

        assert_eq!(packet.active_echoes.len(), 1);
        assert_eq!(packet.active_echoes[0].unchosen_choice_id, "choice:1");
        assert!(packet.tea_recap.remaining_words[0].contains("입 밖으로 나오지 않았다"));
        Ok(())
    }

    #[test]
    fn selected_or_system_slots_do_not_create_echoes() -> anyhow::Result<()> {
        let temp = tempdir()?;
        initialize_hook_ledger_files(temp.path(), "stw_hook", "turn_0000")?;

        let packet = record_offered_choice_set_and_echo(
            temp.path(),
            "stw_hook",
            "turn_0001",
            "place:north_gate",
            &[TurnChoice {
                slot: 6,
                tag: "자유서술".to_owned(),
                intent: "6 뒤에 행동을 쓴다".to_owned(),
            }],
            Some(6),
        )?;

        assert!(packet.active_echoes.is_empty());
        Ok(())
    }

    #[test]
    fn hook_events_require_payoff_and_evidence() {
        let result = accepted_agent_hook_events(&[AgentHookEvent {
            schema_version: HOOK_EVENT_SCHEMA_VERSION.to_owned(),
            event_kind: HookEventKind::Opened,
            hook_id: "hook:north_gate".to_owned(),
            kind: HookKind::Mystery,
            visible_promise: "북문 안쪽 걸쇠는 아직 설명되지 않았다.".to_owned(),
            anchor_refs: vec!["place:north_gate".to_owned()],
            evidence_refs: Vec::new(),
            opened_by_event: "event:north_gate".to_owned(),
            payoff_contract: PayoffContract::default(),
            return_rights: HookReturnRights::default(),
            summary: String::new(),
        }]);

        match result {
            Ok(_) => panic!("missing evidence should fail"),
            Err(err) => assert!(err.to_string().contains("missing evidence_refs")),
        }
    }
}
