// Projection APIs return anyhow::Result with per-call path/context details; the
// Rustdoc error lists would duplicate those local error messages.
#![allow(clippy::missing_errors_doc)]
// Test/projection helpers preserve domain fields explicitly so pressure
// contracts remain readable beside their fixtures.
#![allow(clippy::too_many_arguments)]

use crate::agent_bridge::{AgentPrivateAdjudicationContext, PendingAgentChoice};
use crate::consequence_spine::{ActiveConsequence, ConsequenceKind, ConsequenceSpinePacket};
use crate::extra_memory::ExtraMemoryPacket;
use crate::models::{FREEFORM_CHOICE_SLOT, GUIDE_CHOICE_SLOT, TurnSnapshot};
use crate::social_exchange::{
    DialogueStance, DialogueStanceKind, SocialExchangePacket, SocialIntensity, UnresolvedSocialAsk,
};
use crate::store::{append_jsonl, read_json, write_json};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

pub const SCENE_PRESSURE_PACKET_SCHEMA_VERSION: &str = "singulari.scene_pressure_packet.v1";
pub const SCENE_PRESSURE_SCHEMA_VERSION: &str = "singulari.scene_pressure.v1";
pub const SCENE_PRESSURE_AUDIT_SCHEMA_VERSION: &str = "singulari.scene_pressure_audit.v1";
pub const SCENE_PRESSURE_AUDIT_FILENAME: &str = "scene_pressure_audit.jsonl";
pub const SCENE_PRESSURE_EVENT_SCHEMA_VERSION: &str = "singulari.scene_pressure_event.v1";
pub const SCENE_PRESSURE_EVENTS_FILENAME: &str = "scene_pressure_events.jsonl";
pub const ACTIVE_SCENE_PRESSURES_FILENAME: &str = "active_scene_pressures.json";

const VISIBLE_PRESSURE_BUDGET: usize = 3;
const HIDDEN_PRESSURE_BUDGET: usize = 2;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScenePressurePacket {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub visible_active: Vec<ScenePressure>,
    #[serde(default)]
    pub hidden_adjudication_only: Vec<ScenePressure>,
    pub compiler_policy: ScenePressurePolicy,
}

impl Default for ScenePressurePacket {
    fn default() -> Self {
        Self {
            schema_version: SCENE_PRESSURE_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            turn_id: String::new(),
            visible_active: Vec::new(),
            hidden_adjudication_only: Vec::new(),
            compiler_policy: ScenePressurePolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScenePressure {
    pub schema_version: String,
    pub pressure_id: String,
    pub kind: ScenePressureKind,
    pub visibility: ScenePressureVisibility,
    pub intensity: u8,
    pub urgency: ScenePressureUrgency,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default)]
    pub observable_signals: Vec<String>,
    #[serde(default)]
    pub choice_affordances: Vec<String>,
    pub prose_effect: ScenePressureProseEffect,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScenePressureProseEffect {
    pub paragraph_pressure: String,
    #[serde(default)]
    pub sensory_focus: Vec<String>,
    pub dialogue_style: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScenePressurePolicy {
    pub source: String,
    pub visible_budget: usize,
    pub hidden_budget: usize,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScenePressureAuditRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub compiled_at: String,
    pub source: String,
    pub visible_count: usize,
    pub hidden_count: usize,
    #[serde(default)]
    pub visible_pressure_ids: Vec<String>,
    #[serde(default)]
    pub hidden_pressure_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScenePressureEvent {
    pub pressure_id: String,
    pub change: ScenePressureChange,
    pub intensity_after: u8,
    pub urgency_after: ScenePressureUrgency,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScenePressureChange {
    Surfaced,
    Increased,
    Softened,
    Redirected,
    Resolved,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScenePressureEventRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub event_id: String,
    pub pressure_id: String,
    pub change: ScenePressureChange,
    pub intensity_after: u8,
    pub urgency_after: ScenePressureUrgency,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenePressureEventPlan {
    pub world_id: String,
    pub turn_id: String,
    pub records: Vec<ScenePressureEventRecord>,
}

impl Default for ScenePressurePolicy {
    fn default() -> Self {
        Self {
            source: "compiled_from_pending_turn_v0".to_owned(),
            visible_budget: VISIBLE_PRESSURE_BUDGET,
            hidden_budget: HIDDEN_PRESSURE_BUDGET,
            use_rules: vec![
                "Use active_scene_pressure to shape next_choices and paragraph rhythm.".to_owned(),
                "Do not invent facts from pressure labels; pressure only selects and weights source-backed context.".to_owned(),
                "Hidden pressures may affect adjudication, but must not appear in visible_scene, canon_event, choices, Archive View, or image prompts.".to_owned(),
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScenePressureKind {
    Body,
    Resource,
    TimePressure,
    SocialPermission,
    Threat,
    Knowledge,
    Environment,
    Desire,
    MoralCost,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScenePressureVisibility {
    PlayerVisible,
    Hidden,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScenePressureUrgency {
    Ambient,
    Soon,
    Immediate,
    Crisis,
}

pub fn compile_scene_pressure_packet(
    snapshot: &TurnSnapshot,
    selected_choice: Option<&PendingAgentChoice>,
    extra_memory: &ExtraMemoryPacket,
    private_context: &AgentPrivateAdjudicationContext,
    player_input: &str,
) -> Result<ScenePressurePacket> {
    let mut visible_active = Vec::new();
    collect_choice_pressure(&mut visible_active, selected_choice, player_input);
    collect_body_pressure(&mut visible_active, snapshot);
    collect_knowledge_pressure(&mut visible_active, snapshot);
    collect_extra_social_pressure(&mut visible_active, extra_memory);
    collect_event_pressure(&mut visible_active, snapshot);
    visible_active.truncate(VISIBLE_PRESSURE_BUDGET);

    let mut hidden_adjudication_only = Vec::new();
    collect_hidden_timer_pressure(&mut hidden_adjudication_only, private_context);
    hidden_adjudication_only.truncate(HIDDEN_PRESSURE_BUDGET);

    Ok(ScenePressurePacket {
        schema_version: SCENE_PRESSURE_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: snapshot.world_id.clone(),
        turn_id: next_turn_id(snapshot.turn_id.as_str())?,
        visible_active,
        hidden_adjudication_only,
        compiler_policy: ScenePressurePolicy::default(),
    })
}

pub fn append_scene_pressure_audit(world_dir: &Path, packet: &ScenePressurePacket) -> Result<()> {
    let record = ScenePressureAuditRecord {
        schema_version: SCENE_PRESSURE_AUDIT_SCHEMA_VERSION.to_owned(),
        world_id: packet.world_id.clone(),
        turn_id: packet.turn_id.clone(),
        compiled_at: Utc::now().to_rfc3339(),
        source: packet.compiler_policy.source.clone(),
        visible_count: packet.visible_active.len(),
        hidden_count: packet.hidden_adjudication_only.len(),
        visible_pressure_ids: packet
            .visible_active
            .iter()
            .map(|pressure| pressure.pressure_id.clone())
            .collect(),
        hidden_pressure_ids: packet
            .hidden_adjudication_only
            .iter()
            .map(|pressure| pressure.pressure_id.clone())
            .collect(),
    };
    append_jsonl(&world_dir.join(SCENE_PRESSURE_AUDIT_FILENAME), &record)
}

pub fn prepare_scene_pressure_event_plan(
    packet: &ScenePressurePacket,
    events: &[ScenePressureEvent],
) -> Result<ScenePressureEventPlan> {
    let known_visible_pressures = packet
        .visible_active
        .iter()
        .map(|pressure| pressure.pressure_id.as_str())
        .collect::<BTreeSet<_>>();
    let recorded_at = Utc::now().to_rfc3339();
    let mut records = Vec::with_capacity(events.len());
    for (index, event) in events.iter().enumerate() {
        validate_scene_pressure_event(packet, &known_visible_pressures, event)
            .with_context(|| format!("invalid scene_pressure_events[{index}]"))?;
        records.push(ScenePressureEventRecord {
            schema_version: SCENE_PRESSURE_EVENT_SCHEMA_VERSION.to_owned(),
            world_id: packet.world_id.clone(),
            turn_id: packet.turn_id.clone(),
            event_id: format!("scene_pressure_event:{}:{index:02}", packet.turn_id),
            pressure_id: event.pressure_id.trim().to_owned(),
            change: event.change,
            intensity_after: event.intensity_after,
            urgency_after: event.urgency_after,
            summary: event.summary.trim().to_owned(),
            evidence_refs: event
                .evidence_refs
                .iter()
                .map(|reference| reference.trim().to_owned())
                .collect(),
            recorded_at: recorded_at.clone(),
        });
    }
    Ok(ScenePressureEventPlan {
        world_id: packet.world_id.clone(),
        turn_id: packet.turn_id.clone(),
        records,
    })
}

pub fn append_scene_pressure_event_plan(
    world_dir: &Path,
    plan: &ScenePressureEventPlan,
) -> Result<()> {
    for record in &plan.records {
        append_jsonl(&world_dir.join(SCENE_PRESSURE_EVENTS_FILENAME), record)?;
    }
    Ok(())
}

pub fn rebuild_active_scene_pressures(
    world_dir: &Path,
    base: &ScenePressurePacket,
) -> Result<ScenePressurePacket> {
    let mut packet = base.clone();
    for record in load_scene_pressure_event_records(world_dir)? {
        apply_scene_pressure_record(&mut packet, &record);
    }
    "materialized_from_pending_turn_and_scene_pressure_events_v1"
        .clone_into(&mut packet.compiler_policy.source);
    packet
        .visible_active
        .retain(|pressure| pressure.intensity > 0);
    write_json(&world_dir.join(ACTIVE_SCENE_PRESSURES_FILENAME), &packet)?;
    Ok(packet)
}

pub fn load_active_scene_pressures(
    world_dir: &Path,
    fallback: ScenePressurePacket,
) -> Result<ScenePressurePacket> {
    let path = world_dir.join(ACTIVE_SCENE_PRESSURES_FILENAME);
    if path.exists() {
        return read_json(&path);
    }
    Ok(fallback)
}

#[must_use]
pub fn merge_consequence_scene_pressures(
    mut packet: ScenePressurePacket,
    consequences: &ConsequenceSpinePacket,
) -> ScenePressurePacket {
    let mut existing = packet
        .visible_active
        .iter()
        .map(|pressure| pressure.pressure_id.clone())
        .collect::<BTreeSet<_>>();
    for consequence in &consequences.active {
        if packet.visible_active.len() >= packet.compiler_policy.visible_budget {
            break;
        }
        let pressure = pressure_from_consequence(consequence);
        if existing.insert(pressure.pressure_id.clone()) {
            packet.visible_active.push(pressure);
        }
    }
    if !consequences.active.is_empty() {
        "materialized_from_pending_turn_scene_pressure_and_consequences_v1"
            .clone_into(&mut packet.compiler_policy.source);
    }
    packet
}

#[must_use]
pub fn merge_social_exchange_scene_pressures(
    mut packet: ScenePressurePacket,
    social_exchange: &SocialExchangePacket,
) -> ScenePressurePacket {
    let mut existing = packet
        .visible_active
        .iter()
        .map(|pressure| pressure.pressure_id.clone())
        .collect::<BTreeSet<_>>();
    for stance in &social_exchange.active_stances {
        if packet.visible_active.len() >= packet.compiler_policy.visible_budget {
            break;
        }
        if !stance_creates_pressure(stance) {
            continue;
        }
        let pressure = pressure_from_stance(stance);
        if existing.insert(pressure.pressure_id.clone()) {
            packet.visible_active.push(pressure);
        }
    }
    for ask in &social_exchange.unresolved_asks {
        if packet.visible_active.len() >= packet.compiler_policy.visible_budget {
            break;
        }
        let pressure = pressure_from_unresolved_ask(ask);
        if existing.insert(pressure.pressure_id.clone()) {
            packet.visible_active.push(pressure);
        }
    }
    if !social_exchange.active_stances.is_empty() || !social_exchange.unresolved_asks.is_empty() {
        "materialized_from_pending_turn_scene_pressure_consequences_and_social_exchange_v1"
            .clone_into(&mut packet.compiler_policy.source);
    }
    packet
}

fn stance_creates_pressure(stance: &DialogueStance) -> bool {
    matches!(
        stance.intensity,
        SocialIntensity::Medium | SocialIntensity::High | SocialIntensity::Crisis
    ) && matches!(
        stance.stance,
        DialogueStanceKind::WaryTesting
            | DialogueStanceKind::Evasive
            | DialogueStanceKind::Threatening
            | DialogueStanceKind::Bargaining
            | DialogueStanceKind::Indebted
            | DialogueStanceKind::Pressuring
            | DialogueStanceKind::Withholding
    )
}

fn pressure_from_stance(stance: &DialogueStance) -> ScenePressure {
    ScenePressure {
        schema_version: SCENE_PRESSURE_SCHEMA_VERSION.to_owned(),
        pressure_id: format!("pressure:social_stance:{}", stance.stance_id),
        kind: ScenePressureKind::SocialPermission,
        visibility: ScenePressureVisibility::PlayerVisible,
        intensity: pressure_intensity_from_social(stance.intensity),
        urgency: pressure_urgency_from_social(stance.intensity),
        source_refs: stance.source_refs.clone(),
        observable_signals: vec![stance.player_visible_signal.clone()],
        choice_affordances: social_choice_affordances(stance.stance),
        prose_effect: ScenePressureProseEffect {
            paragraph_pressure: "social_exchange".to_owned(),
            sensory_focus: vec!["거리".to_owned(), "대사".to_owned()],
            dialogue_style: dialogue_style_from_stance(stance.stance),
        },
    }
}

fn pressure_from_unresolved_ask(ask: &UnresolvedSocialAsk) -> ScenePressure {
    ScenePressure {
        schema_version: SCENE_PRESSURE_SCHEMA_VERSION.to_owned(),
        pressure_id: format!("pressure:unresolved_ask:{}", ask.ask_id),
        kind: ScenePressureKind::Knowledge,
        visibility: ScenePressureVisibility::PlayerVisible,
        intensity: 3,
        urgency: ScenePressureUrgency::Soon,
        source_refs: ask.source_refs.clone(),
        observable_signals: vec![ask.last_response.clone()],
        choice_affordances: ask.allowed_next_moves.clone(),
        prose_effect: ScenePressureProseEffect {
            paragraph_pressure: "unresolved_question".to_owned(),
            sensory_focus: vec!["침묵".to_owned(), "시선".to_owned()],
            dialogue_style: "avoid_repeat_without_new_leverage".to_owned(),
        },
    }
}

const fn pressure_intensity_from_social(intensity: SocialIntensity) -> u8 {
    match intensity {
        SocialIntensity::Trace => 1,
        SocialIntensity::Low => 2,
        SocialIntensity::Medium => 3,
        SocialIntensity::High => 4,
        SocialIntensity::Crisis => 5,
    }
}

const fn pressure_urgency_from_social(intensity: SocialIntensity) -> ScenePressureUrgency {
    match intensity {
        SocialIntensity::Trace | SocialIntensity::Low => ScenePressureUrgency::Ambient,
        SocialIntensity::Medium => ScenePressureUrgency::Soon,
        SocialIntensity::High => ScenePressureUrgency::Immediate,
        SocialIntensity::Crisis => ScenePressureUrgency::Crisis,
    }
}

fn social_choice_affordances(stance: DialogueStanceKind) -> Vec<String> {
    match stance {
        DialogueStanceKind::WaryTesting => vec!["신원, 근거, 의도를 분명히 밝힌다.".to_owned()],
        DialogueStanceKind::Evasive | DialogueStanceKind::Withholding => {
            vec!["새 근거를 제시하거나 질문의 각도를 바꾼다.".to_owned()]
        }
        DialogueStanceKind::Threatening | DialogueStanceKind::Pressuring => {
            vec!["압박에 맞서거나 한발 물러서 조건을 바꾼다.".to_owned()]
        }
        DialogueStanceKind::Bargaining => vec!["대가, 조건, 양보안을 조정한다.".to_owned()],
        DialogueStanceKind::Indebted => vec!["남은 빚이나 호의를 구체적으로 건드린다.".to_owned()],
        _ => vec!["현재 대화 거리와 태도에 맞춰 말한다.".to_owned()],
    }
}

fn dialogue_style_from_stance(stance: DialogueStanceKind) -> String {
    match stance {
        DialogueStanceKind::WaryTesting => "testing".to_owned(),
        DialogueStanceKind::Evasive | DialogueStanceKind::Withholding => "evasive".to_owned(),
        DialogueStanceKind::Threatening | DialogueStanceKind::Pressuring => "pressuring".to_owned(),
        DialogueStanceKind::Bargaining => "bargaining".to_owned(),
        DialogueStanceKind::Indebted => "debt-aware".to_owned(),
        _ => "socially-aware".to_owned(),
    }
}

fn pressure_from_consequence(consequence: &ActiveConsequence) -> ScenePressure {
    ScenePressure {
        schema_version: SCENE_PRESSURE_SCHEMA_VERSION.to_owned(),
        pressure_id: format!("pressure:consequence:{}", consequence.consequence_id),
        kind: pressure_kind_from_consequence(consequence.kind),
        visibility: ScenePressureVisibility::PlayerVisible,
        intensity: pressure_intensity_from_consequence(consequence.severity),
        urgency: pressure_urgency_from_consequence(consequence.severity),
        source_refs: vec![consequence.consequence_id.clone()],
        observable_signals: vec![consequence.player_visible_signal.clone()],
        choice_affordances: vec!["이전 선택의 여파를 감안해 행동한다.".to_owned()],
        prose_effect: ScenePressureProseEffect {
            paragraph_pressure: "consequence".to_owned(),
            sensory_focus: vec!["aftermath".to_owned()],
            dialogue_style: "consequence-aware".to_owned(),
        },
    }
}

const fn pressure_kind_from_consequence(kind: ConsequenceKind) -> ScenePressureKind {
    match kind {
        ConsequenceKind::BodyCost => ScenePressureKind::Body,
        ConsequenceKind::ResourceCost => ScenePressureKind::Resource,
        ConsequenceKind::SocialDebt
        | ConsequenceKind::TrustShift
        | ConsequenceKind::SuspicionRaised => ScenePressureKind::SocialPermission,
        ConsequenceKind::AlarmRaised => ScenePressureKind::Threat,
        ConsequenceKind::KnowledgeOpened | ConsequenceKind::KnowledgeResolved => {
            ScenePressureKind::Knowledge
        }
        ConsequenceKind::LocationAccessChanged => ScenePressureKind::Environment,
        ConsequenceKind::ProcessAccelerated | ConsequenceKind::ProcessDelayed => {
            ScenePressureKind::TimePressure
        }
        ConsequenceKind::MoralDebt => ScenePressureKind::MoralCost,
        ConsequenceKind::OpportunityOpened | ConsequenceKind::OpportunityLost => {
            ScenePressureKind::Desire
        }
    }
}

const fn pressure_intensity_from_consequence(
    severity: crate::consequence_spine::ConsequenceSeverity,
) -> u8 {
    match severity {
        crate::consequence_spine::ConsequenceSeverity::Trace => 1,
        crate::consequence_spine::ConsequenceSeverity::Minor => 2,
        crate::consequence_spine::ConsequenceSeverity::Moderate => 3,
        crate::consequence_spine::ConsequenceSeverity::Major => 4,
        crate::consequence_spine::ConsequenceSeverity::Critical => 5,
    }
}

const fn pressure_urgency_from_consequence(
    severity: crate::consequence_spine::ConsequenceSeverity,
) -> ScenePressureUrgency {
    match severity {
        crate::consequence_spine::ConsequenceSeverity::Trace => ScenePressureUrgency::Ambient,
        crate::consequence_spine::ConsequenceSeverity::Minor
        | crate::consequence_spine::ConsequenceSeverity::Moderate => ScenePressureUrgency::Soon,
        crate::consequence_spine::ConsequenceSeverity::Major => ScenePressureUrgency::Immediate,
        crate::consequence_spine::ConsequenceSeverity::Critical => ScenePressureUrgency::Crisis,
    }
}

fn load_scene_pressure_event_records(world_dir: &Path) -> Result<Vec<ScenePressureEventRecord>> {
    let path = world_dir.join(SCENE_PRESSURE_EVENTS_FILENAME);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)?;
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str::<ScenePressureEventRecord>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn apply_scene_pressure_record(
    packet: &mut ScenePressurePacket,
    record: &ScenePressureEventRecord,
) {
    if let Some(pressure) = packet
        .visible_active
        .iter_mut()
        .find(|pressure| pressure.pressure_id == record.pressure_id)
    {
        pressure.intensity = if matches!(record.change, ScenePressureChange::Resolved) {
            0
        } else {
            record.intensity_after
        };
        pressure.urgency = record.urgency_after;
        pressure.observable_signals = vec![record.summary.clone()];
        pressure
            .source_refs
            .push(format!("scene_pressure_event:{}", record.event_id));
    }
}

fn validate_scene_pressure_event(
    packet: &ScenePressurePacket,
    known_visible_pressures: &BTreeSet<&str>,
    event: &ScenePressureEvent,
) -> Result<()> {
    let pressure_id = event.pressure_id.trim();
    if pressure_id.is_empty() {
        bail!("scene pressure event pressure_id must not be empty");
    }
    if !known_visible_pressures.contains(pressure_id) {
        bail!(
            "scene pressure event references inactive visible pressure: world_id={}, turn_id={}, pressure_id={pressure_id}",
            packet.world_id,
            packet.turn_id
        );
    }
    if event.intensity_after == 0 || event.intensity_after > 5 {
        bail!("scene pressure event intensity_after must be 1..5");
    }
    if event.summary.trim().is_empty() {
        bail!("scene pressure event summary must not be empty");
    }
    if event.evidence_refs.is_empty()
        || event
            .evidence_refs
            .iter()
            .any(|reference| reference.trim().is_empty())
    {
        bail!("scene pressure event evidence_refs must contain non-empty visible refs");
    }
    Ok(())
}

fn collect_choice_pressure(
    pressures: &mut Vec<ScenePressure>,
    selected_choice: Option<&PendingAgentChoice>,
    player_input: &str,
) {
    let Some(choice) = selected_choice else {
        if !player_input.trim().is_empty() {
            pressures.push(visible_pressure(
                "player_free_input",
                ScenePressureKind::Knowledge,
                2,
                ScenePressureUrgency::Immediate,
                vec!["player_input".to_owned()],
                vec!["player supplied a freeform action or world-opening input".to_owned()],
                vec![
                    "answer the action directly".to_owned(),
                    "preserve uncertainty".to_owned(),
                ],
                prose("focused", vec!["hands", "nearby reaction"], "action-first"),
            ));
        }
        return;
    };
    let (kind, affordance) = match choice.slot {
        GUIDE_CHOICE_SLOT => (
            ScenePressureKind::Knowledge,
            "resolve delegated judgment without exposing hidden route",
        ),
        FREEFORM_CHOICE_SLOT => (
            ScenePressureKind::Knowledge,
            "adjudicate the inline freeform action against visible constraints",
        ),
        _ => (
            ScenePressureKind::SocialPermission,
            "continue from selected visible intent",
        ),
    };
    pressures.push(visible_pressure(
        format!("choice_slot_{}", choice.slot).as_str(),
        kind,
        3,
        ScenePressureUrgency::Immediate,
        vec![format!("choice:slot:{}", choice.slot)],
        vec![format!(
            "selected choice: {} / {}",
            choice.tag, choice.visible_intent
        )],
        vec![affordance.to_owned()],
        prose(
            "tight",
            vec!["choice consequence", "nearest witness"],
            "scene-specific",
        ),
    ));
}

fn collect_body_pressure(pressures: &mut Vec<ScenePressure>, snapshot: &TurnSnapshot) {
    if snapshot.protagonist_state.body.is_empty() {
        return;
    }
    pressures.push(visible_pressure(
        "protagonist_body",
        ScenePressureKind::Body,
        2,
        ScenePressureUrgency::Soon,
        vec!["latest_snapshot.protagonist_state.body".to_owned()],
        snapshot.protagonist_state.body.clone(),
        vec!["make physical condition matter if the action strains it".to_owned()],
        prose("friction", vec!["breath", "grip", "posture"], "restrained"),
    ));
}

fn collect_knowledge_pressure(pressures: &mut Vec<ScenePressure>, snapshot: &TurnSnapshot) {
    if snapshot.open_questions.is_empty() && snapshot.protagonist_state.mind.is_empty() {
        return;
    }
    let mut signals = snapshot
        .open_questions
        .iter()
        .map(|question| format!("open question: {question}"))
        .collect::<Vec<_>>();
    signals.extend(
        snapshot
            .protagonist_state
            .mind
            .iter()
            .map(|mind| format!("mind: {mind}")),
    );
    pressures.push(visible_pressure(
        "open_questions",
        ScenePressureKind::Knowledge,
        2,
        ScenePressureUrgency::Soon,
        vec![
            "latest_snapshot.open_questions".to_owned(),
            "latest_snapshot.protagonist_state.mind".to_owned(),
        ],
        signals,
        vec!["preserve unresolved knowledge instead of explaining hidden truth".to_owned()],
        prose(
            "unresolved",
            vec!["unexplained detail", "withheld interpretation"],
            "low-exposition",
        ),
    ));
}

fn collect_extra_social_pressure(
    pressures: &mut Vec<ScenePressure>,
    extra_memory: &ExtraMemoryPacket,
) {
    let remembered = extra_memory.remembered_extras.first();
    let recent = extra_memory.recent_extra_traces.first();
    if remembered.is_none() && recent.is_none() {
        return;
    }
    let mut signals = Vec::new();
    let mut refs = Vec::new();
    if let Some(extra) = remembered {
        refs.push(format!("remembered_extra:{}", extra.extra_id));
        signals.push(format!(
            "local face: {} / {}",
            extra.display_name, extra.disposition
        ));
        if let Some(hook) = extra.open_hooks.first() {
            signals.push(format!("open hook: {hook}"));
        }
    }
    if let Some(trace) = recent {
        refs.push(format!("extra_trace:{}", trace.trace_id));
        signals.push(format!(
            "recent contact: {} / {}",
            trace.surface_label, trace.contact_summary
        ));
    }
    pressures.push(visible_pressure(
        "local_faces",
        ScenePressureKind::SocialPermission,
        2,
        ScenePressureUrgency::Soon,
        refs,
        signals,
        vec![
            "let remembered local faces react only if the scene naturally touches them".to_owned(),
        ],
        prose(
            "social",
            vec!["gaze", "distance", "small recognition"],
            "relationship-aware",
        ),
    ));
}

fn collect_event_pressure(pressures: &mut Vec<ScenePressure>, snapshot: &TurnSnapshot) {
    let Some(event) = &snapshot.current_event else {
        return;
    };
    pressures.push(visible_pressure(
        "current_event",
        ScenePressureKind::TimePressure,
        2,
        ScenePressureUrgency::Soon,
        vec![format!("current_event:{}", event.event_id)],
        vec![format!("event progress: {}", event.progress)],
        vec!["keep the current event moving instead of resetting setup".to_owned()],
        prose(
            "forward",
            vec!["elapsed time", "nearby movement"],
            "compressed",
        ),
    ));
}

fn collect_hidden_timer_pressure(
    pressures: &mut Vec<ScenePressure>,
    private_context: &AgentPrivateAdjudicationContext,
) {
    for timer in private_context
        .hidden_timers
        .iter()
        .filter(|timer| timer.remaining_turns <= 2)
    {
        pressures.push(ScenePressure {
            schema_version: SCENE_PRESSURE_SCHEMA_VERSION.to_owned(),
            pressure_id: format!("pressure:hidden_timer:{}", timer.timer_id),
            kind: ScenePressureKind::TimePressure,
            visibility: ScenePressureVisibility::Hidden,
            intensity: if timer.remaining_turns == 0 { 5 } else { 4 },
            urgency: if timer.remaining_turns == 0 {
                ScenePressureUrgency::Crisis
            } else {
                ScenePressureUrgency::Immediate
            },
            source_refs: vec![format!("hidden_timer:{}", timer.timer_id)],
            observable_signals: Vec::new(),
            choice_affordances: vec![
                "adjudication-only deadline; do not reveal timer truth in visible text".to_owned(),
            ],
            prose_effect: prose("omission", Vec::new(), "do-not-reveal"),
        });
    }
}

fn visible_pressure(
    id_suffix: &str,
    kind: ScenePressureKind,
    intensity: u8,
    urgency: ScenePressureUrgency,
    source_refs: Vec<String>,
    observable_signals: Vec<String>,
    choice_affordances: Vec<String>,
    prose_effect: ScenePressureProseEffect,
) -> ScenePressure {
    ScenePressure {
        schema_version: SCENE_PRESSURE_SCHEMA_VERSION.to_owned(),
        pressure_id: format!("pressure:{id_suffix}"),
        kind,
        visibility: ScenePressureVisibility::PlayerVisible,
        intensity,
        urgency,
        source_refs,
        observable_signals,
        choice_affordances,
        prose_effect,
    }
}

fn prose(
    paragraph_pressure: &str,
    sensory_focus: Vec<&str>,
    dialogue_style: &str,
) -> ScenePressureProseEffect {
    ScenePressureProseEffect {
        paragraph_pressure: paragraph_pressure.to_owned(),
        sensory_focus: sensory_focus
            .into_iter()
            .map(std::borrow::ToOwned::to_owned)
            .collect(),
        dialogue_style: dialogue_style.to_owned(),
    }
}

fn next_turn_id(current: &str) -> Result<String> {
    let Some(number) = current.strip_prefix("turn_") else {
        bail!("scene pressure snapshot turn_id must start with turn_: actual={current}");
    };
    let number = number.parse::<u32>().with_context(|| {
        format!("scene pressure snapshot turn_id has invalid numeric suffix: {current}")
    })?;
    Ok(format!("turn_{:04}", number + 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_bridge::{AgentHiddenTimer, AgentPrivateAdjudicationContext};
    use crate::models::{ProtagonistState, TURN_SNAPSHOT_SCHEMA_VERSION, TurnSnapshot};

    #[test]
    fn compiles_visible_body_and_knowledge_pressures() -> anyhow::Result<()> {
        let snapshot = TurnSnapshot {
            schema_version: TURN_SNAPSHOT_SCHEMA_VERSION.to_owned(),
            world_id: "stw_pressure".to_owned(),
            session_id: "session".to_owned(),
            turn_id: "turn_0003".to_owned(),
            phase: "choice".to_owned(),
            current_event: None,
            protagonist_state: ProtagonistState {
                location: "place:gate".to_owned(),
                inventory: Vec::new(),
                body: vec!["left wrist aches".to_owned()],
                mind: vec!["does not know who controls the gate".to_owned()],
            },
            open_questions: vec!["why is the gate closing early?".to_owned()],
            last_choices: Vec::new(),
        };

        let packet = compile_scene_pressure_packet(
            &snapshot,
            None,
            &ExtraMemoryPacket::default(),
            &AgentPrivateAdjudicationContext {
                hidden_timers: Vec::new(),
                unrevealed_constraints: Vec::new(),
                plausibility_gates: Vec::new(),
            },
            "주변을 살핀다",
        )?;

        assert_eq!(packet.turn_id, "turn_0004");
        assert!(
            packet
                .visible_active
                .iter()
                .any(|pressure| pressure.kind == ScenePressureKind::Body)
        );
        assert!(
            packet
                .visible_active
                .iter()
                .any(|pressure| pressure.kind == ScenePressureKind::Knowledge)
        );
        assert!(packet.hidden_adjudication_only.is_empty());
        Ok(())
    }

    #[test]
    fn hidden_timer_pressure_has_no_observable_signals() -> anyhow::Result<()> {
        let snapshot = TurnSnapshot {
            schema_version: TURN_SNAPSHOT_SCHEMA_VERSION.to_owned(),
            world_id: "stw_pressure".to_owned(),
            session_id: "session".to_owned(),
            turn_id: "turn_0000".to_owned(),
            phase: "choice".to_owned(),
            current_event: None,
            protagonist_state: ProtagonistState {
                location: "place:gate".to_owned(),
                inventory: Vec::new(),
                body: Vec::new(),
                mind: Vec::new(),
            },
            open_questions: Vec::new(),
            last_choices: Vec::new(),
        };
        let private = AgentPrivateAdjudicationContext {
            hidden_timers: vec![AgentHiddenTimer {
                timer_id: "timer:pursuit".to_owned(),
                kind: "pursuit".to_owned(),
                remaining_turns: 1,
                effect: "patrol arrives".to_owned(),
            }],
            unrevealed_constraints: Vec::new(),
            plausibility_gates: Vec::new(),
        };

        let packet = compile_scene_pressure_packet(
            &snapshot,
            None,
            &ExtraMemoryPacket::default(),
            &private,
            "1",
        )?;

        assert_eq!(packet.hidden_adjudication_only.len(), 1);
        assert_eq!(
            packet.hidden_adjudication_only[0].visibility,
            ScenePressureVisibility::Hidden
        );
        assert!(
            packet.hidden_adjudication_only[0]
                .observable_signals
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn active_consequence_returns_as_visible_scene_pressure() {
        let packet = ScenePressurePacket {
            world_id: "world".to_owned(),
            turn_id: "turn_0004".to_owned(),
            ..ScenePressurePacket::default()
        };
        let merged = merge_consequence_scene_pressures(
            packet,
            &ConsequenceSpinePacket {
                world_id: "world".to_owned(),
                turn_id: "turn_0004".to_owned(),
                active: vec![ActiveConsequence {
                    schema_version: crate::consequence_spine::CONSEQUENCE_SCHEMA_VERSION.to_owned(),
                    consequence_id: "consequence:turn_0003:gate".to_owned(),
                    origin_turn_id: "turn_0003".to_owned(),
                    kind: ConsequenceKind::SuspicionRaised,
                    scope: crate::consequence_spine::ConsequenceScope::Relationship,
                    status: crate::consequence_spine::ConsequenceStatus::Active,
                    severity: crate::consequence_spine::ConsequenceSeverity::Major,
                    summary: "문지기의 의심이 남았다.".to_owned(),
                    player_visible_signal: "문답은 의심에서 시작된다.".to_owned(),
                    source_refs: vec!["choice:1".to_owned()],
                    linked_entity_refs: Vec::new(),
                    linked_projection_refs: vec!["rel:guard".to_owned()],
                    expected_return:
                        crate::consequence_spine::ConsequenceReturnWindow::CurrentScene,
                    decay: crate::consequence_spine::ConsequenceDecay::default(),
                }],
                ..ConsequenceSpinePacket::default()
            },
        );

        assert_eq!(merged.visible_active.len(), 1);
        assert_eq!(
            merged.visible_active[0].kind,
            ScenePressureKind::SocialPermission
        );
        assert_eq!(
            merged.visible_active[0].urgency,
            ScenePressureUrgency::Immediate
        );
    }

    #[test]
    fn active_social_exchange_returns_as_visible_scene_pressure() {
        let merged = merge_social_exchange_scene_pressures(
            ScenePressurePacket {
                world_id: "stw_pressure".to_owned(),
                turn_id: "turn_0004".to_owned(),
                ..ScenePressurePacket::default()
            },
            &crate::social_exchange::SocialExchangePacket {
                world_id: "stw_pressure".to_owned(),
                turn_id: "turn_0004".to_owned(),
                active_stances: vec![crate::social_exchange::DialogueStance {
                    schema_version: crate::social_exchange::SOCIAL_EXCHANGE_STANCE_SCHEMA_VERSION
                        .to_owned(),
                    stance_id: "stance:char:guard->player".to_owned(),
                    actor_ref: "char:guard".to_owned(),
                    target_ref: "player".to_owned(),
                    stance: DialogueStanceKind::Withholding,
                    intensity: SocialIntensity::High,
                    summary: "문지기가 답을 미룬다.".to_owned(),
                    player_visible_signal: "신원 확인 전에는 답하지 않는다.".to_owned(),
                    source_refs: vec!["visible_scene.text_blocks[0]".to_owned()],
                    relationship_refs: Vec::new(),
                    consequence_refs: Vec::new(),
                    opened_turn_id: "turn_0003".to_owned(),
                    last_changed_turn_id: "turn_0004".to_owned(),
                }],
                ..crate::social_exchange::SocialExchangePacket::default()
            },
        );

        assert_eq!(merged.visible_active.len(), 1);
        assert_eq!(
            merged.visible_active[0].kind,
            ScenePressureKind::SocialPermission
        );
        assert!(
            merged.visible_active[0]
                .pressure_id
                .starts_with("pressure:social_stance:")
        );
    }

    #[test]
    fn prepares_scene_pressure_event_records_for_visible_pressure() -> anyhow::Result<()> {
        let snapshot = TurnSnapshot {
            schema_version: TURN_SNAPSHOT_SCHEMA_VERSION.to_owned(),
            world_id: "stw_pressure".to_owned(),
            session_id: "session".to_owned(),
            turn_id: "turn_0003".to_owned(),
            phase: "choice".to_owned(),
            current_event: None,
            protagonist_state: ProtagonistState {
                location: "place:gate".to_owned(),
                inventory: Vec::new(),
                body: vec!["left wrist aches".to_owned()],
                mind: Vec::new(),
            },
            open_questions: Vec::new(),
            last_choices: Vec::new(),
        };
        let packet = compile_scene_pressure_packet(
            &snapshot,
            None,
            &ExtraMemoryPacket::default(),
            &AgentPrivateAdjudicationContext {
                hidden_timers: Vec::new(),
                unrevealed_constraints: Vec::new(),
                plausibility_gates: Vec::new(),
            },
            "listen",
        )?;

        let plan = prepare_scene_pressure_event_plan(
            &packet,
            &[ScenePressureEvent {
                pressure_id: packet.visible_active[0].pressure_id.clone(),
                change: ScenePressureChange::Increased,
                intensity_after: 3,
                urgency_after: ScenePressureUrgency::Soon,
                summary: "the visible pressure shaped the next beat".to_owned(),
                evidence_refs: vec!["visible_scene.text_blocks[0]".to_owned()],
            }],
        )?;

        assert_eq!(plan.records.len(), 1);
        assert!(
            plan.records[0]
                .event_id
                .starts_with("scene_pressure_event:")
        );
        Ok(())
    }

    #[test]
    fn rejects_scene_pressure_events_for_hidden_pressure() {
        let mut packet = ScenePressurePacket {
            world_id: "stw_pressure".to_owned(),
            turn_id: "turn_0001".to_owned(),
            ..ScenePressurePacket::default()
        };
        packet.hidden_adjudication_only.push(ScenePressure {
            schema_version: SCENE_PRESSURE_SCHEMA_VERSION.to_owned(),
            pressure_id: "pressure:hidden:timer_hidden".to_owned(),
            kind: ScenePressureKind::TimePressure,
            visibility: ScenePressureVisibility::Hidden,
            intensity: 3,
            urgency: ScenePressureUrgency::Soon,
            source_refs: vec!["timer:private".to_owned()],
            observable_signals: Vec::new(),
            choice_affordances: Vec::new(),
            prose_effect: prose("tight", vec!["clock"], "restrained"),
        });

        let Err(error) = prepare_scene_pressure_event_plan(
            &packet,
            &[ScenePressureEvent {
                pressure_id: "pressure:hidden:timer_hidden".to_owned(),
                change: ScenePressureChange::Resolved,
                intensity_after: 1,
                urgency_after: ScenePressureUrgency::Ambient,
                summary: "hidden timer resolved".to_owned(),
                evidence_refs: vec!["visible_scene.text_blocks[0]".to_owned()],
            }],
        ) else {
            panic!("hidden pressure id must reject scene pressure event");
        };

        assert!(
            error
                .to_string()
                .contains("invalid scene_pressure_events[0]")
        );
    }
}
