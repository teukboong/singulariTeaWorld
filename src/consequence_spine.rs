#![allow(clippy::missing_errors_doc)]

use crate::prompt_context::PromptContextPacket;
use crate::resolution::{
    GateKind, GateStatus, ProposedEffectKind, ResolutionOutcomeKind, ResolutionProposal,
    ResolutionVisibility,
};
use crate::social_exchange::{
    DialogueStanceKind, SocialCommitmentKind, SocialExchangeProposal, SocialIntensity,
};
use crate::store::{append_jsonl, read_json, write_json};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const CONSEQUENCE_SPINE_PACKET_SCHEMA_VERSION: &str = "singulari.consequence_spine_packet.v1";
pub const CONSEQUENCE_SCHEMA_VERSION: &str = "singulari.consequence.v1";
pub const CONSEQUENCE_EVENT_SCHEMA_VERSION: &str = "singulari.consequence_event.v1";
pub const CONSEQUENCE_PROPOSAL_SCHEMA_VERSION: &str = "singulari.consequence_proposal.v1";
pub const CONSEQUENCE_EVENTS_FILENAME: &str = "consequence_events.jsonl";
pub const ACTIVE_CONSEQUENCES_FILENAME: &str = "active_consequences.json";

const ACTIVE_CONSEQUENCE_BUDGET: usize = 8;
const PAID_OFF_MEMORY_BUDGET: usize = 5;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsequenceSpinePacket {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub active: Vec<ActiveConsequence>,
    #[serde(default)]
    pub recently_paid_off: Vec<ConsequenceMemory>,
    #[serde(default)]
    pub pressure_links: Vec<ConsequencePressureLink>,
    #[serde(default)]
    pub required_followups: Vec<ConsequenceFollowup>,
    pub compiler_policy: ConsequenceSpinePolicy,
}

impl Default for ConsequenceSpinePacket {
    fn default() -> Self {
        Self {
            schema_version: CONSEQUENCE_SPINE_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            turn_id: String::new(),
            active: Vec::new(),
            recently_paid_off: Vec::new(),
            pressure_links: Vec::new(),
            required_followups: Vec::new(),
            compiler_policy: ConsequenceSpinePolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveConsequence {
    pub schema_version: String,
    pub consequence_id: String,
    pub origin_turn_id: String,
    pub kind: ConsequenceKind,
    pub scope: ConsequenceScope,
    pub status: ConsequenceStatus,
    pub severity: ConsequenceSeverity,
    pub summary: String,
    pub player_visible_signal: String,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default)]
    pub linked_entity_refs: Vec<String>,
    #[serde(default)]
    pub linked_projection_refs: Vec<String>,
    pub expected_return: ConsequenceReturnWindow,
    pub decay: ConsequenceDecay,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsequenceMemory {
    pub consequence_id: String,
    pub kind: ConsequenceKind,
    pub summary: String,
    pub payoff_summary: String,
    #[serde(default)]
    pub source_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsequencePressureLink {
    pub consequence_id: String,
    pub pressure_kind: String,
    pub observable_signal: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsequenceFollowup {
    pub followup_id: String,
    pub reason: String,
    #[serde(default)]
    pub source_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsequenceSpinePolicy {
    pub source: String,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for ConsequenceSpinePolicy {
    fn default() -> Self {
        Self {
            source: "materialized_from_consequence_events_v1".to_owned(),
            use_rules: vec![
                "Use active consequences to make prior choices return as pressure, not as exposition.".to_owned(),
                "Do not treat consequence labels as hidden future truth or route guidance.".to_owned(),
                "Player-visible summaries must describe only what the player can fairly know.".to_owned(),
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ConsequenceKind {
    BodyCost,
    ResourceCost,
    SocialDebt,
    TrustShift,
    SuspicionRaised,
    AlarmRaised,
    KnowledgeOpened,
    KnowledgeResolved,
    LocationAccessChanged,
    ProcessAccelerated,
    ProcessDelayed,
    MoralDebt,
    OpportunityOpened,
    OpportunityLost,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConsequenceScope {
    Body,
    Inventory,
    Relationship,
    Location,
    Faction,
    Knowledge,
    WorldProcess,
    Scene,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ConsequenceSeverity {
    Trace,
    Minor,
    Moderate,
    Major,
    Critical,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConsequenceStatus {
    Active,
    Escalated,
    Softened,
    Transferred,
    PaidOff,
    Decayed,
    Superseded,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConsequenceReturnWindow {
    SearchOnly,
    WhenRelevant,
    Soon,
    NextTurn,
    CurrentScene,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsequenceDecay {
    pub mode: String,
    pub remaining_turns: u8,
    pub payoff_hint: String,
}

impl Default for ConsequenceDecay {
    fn default() -> Self {
        Self {
            mode: "relevance".to_owned(),
            remaining_turns: 3,
            payoff_hint: "Escalate, soften, pay off, or let it decay when no longer relevant."
                .to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsequenceProposal {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub introduced: Vec<ConsequenceMutation>,
    #[serde(default)]
    pub updated: Vec<ConsequenceMutation>,
    #[serde(default)]
    pub paid_off: Vec<ConsequencePayoff>,
    #[serde(default)]
    pub ephemeral_effects: Vec<EphemeralConsequenceReason>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsequenceMutation {
    pub consequence_id: String,
    pub kind: ConsequenceKind,
    pub scope: ConsequenceScope,
    pub severity: ConsequenceSeverity,
    pub summary: String,
    pub player_visible_signal: String,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default)]
    pub linked_entity_refs: Vec<String>,
    #[serde(default)]
    pub linked_projection_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsequencePayoff {
    pub consequence_id: String,
    pub payoff_summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EphemeralConsequenceReason {
    pub effect_ref: String,
    pub reason: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsequenceEventPlan {
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub records: Vec<ConsequenceEventRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsequenceEventRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub event_id: String,
    pub consequence_id: String,
    pub event_kind: ConsequenceEventKind,
    pub kind: ConsequenceKind,
    pub scope: ConsequenceScope,
    pub severity: ConsequenceSeverity,
    pub summary: String,
    pub player_visible_signal: String,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default)]
    pub linked_entity_refs: Vec<String>,
    #[serde(default)]
    pub linked_projection_refs: Vec<String>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConsequenceEventKind {
    Introduced,
    Escalated,
    Softened,
    Transferred,
    PaidOff,
    Decayed,
    Superseded,
}

pub fn prepare_consequence_event_plan(
    current: &ConsequenceSpinePacket,
    proposal: Option<&ConsequenceProposal>,
    resolution: Option<&ResolutionProposal>,
    social_exchange: Option<&SocialExchangeProposal>,
) -> Result<ConsequenceEventPlan> {
    let mut records = Vec::new();
    let recorded_at = Utc::now().to_rfc3339();
    if let Some(proposal) = proposal {
        validate_consequence_proposal(current, proposal)?;
        for mutation in proposal.introduced.iter().chain(proposal.updated.iter()) {
            let event_kind = if current
                .active
                .iter()
                .any(|active| active.consequence_id == mutation.consequence_id)
            {
                ConsequenceEventKind::Escalated
            } else {
                ConsequenceEventKind::Introduced
            };
            records.push(record_from_mutation(
                current,
                mutation,
                event_kind,
                records.len(),
                recorded_at.as_str(),
            ));
        }
        for payoff in &proposal.paid_off {
            let active = current
                .active
                .iter()
                .find(|active| active.consequence_id == payoff.consequence_id)
                .with_context(|| {
                    format!(
                        "consequence payoff references inactive consequence: {}",
                        payoff.consequence_id
                    )
                })?;
            if payoff.evidence_refs.is_empty() {
                bail!("consequence payoff requires evidence_refs");
            }
            records.push(ConsequenceEventRecord {
                schema_version: CONSEQUENCE_EVENT_SCHEMA_VERSION.to_owned(),
                world_id: current.world_id.clone(),
                turn_id: current.turn_id.clone(),
                event_id: format!("consequence_event:{}:{:02}", current.turn_id, records.len()),
                consequence_id: payoff.consequence_id.clone(),
                event_kind: ConsequenceEventKind::PaidOff,
                kind: active.kind,
                scope: active.scope,
                severity: active.severity,
                summary: payoff.payoff_summary.clone(),
                player_visible_signal: payoff.payoff_summary.clone(),
                source_refs: payoff.evidence_refs.clone(),
                linked_entity_refs: active.linked_entity_refs.clone(),
                linked_projection_refs: active.linked_projection_refs.clone(),
                recorded_at: recorded_at.clone(),
            });
        }
    }
    if let Some(resolution) = resolution {
        let existing_refs = records
            .iter()
            .flat_map(|record| record.source_refs.iter().map(String::as_str))
            .collect::<BTreeSet<_>>();
        let mut derived = derive_consequence_records(
            current,
            resolution,
            records.len(),
            &existing_refs,
            recorded_at.as_str(),
        );
        records.append(&mut derived);
    }
    if let Some(social_exchange) = social_exchange {
        let existing_refs = records
            .iter()
            .flat_map(|record| record.source_refs.iter().map(String::as_str))
            .collect::<BTreeSet<_>>();
        let mut derived = derive_consequence_records_from_social_exchange(
            current,
            social_exchange,
            records.len(),
            &existing_refs,
            recorded_at.as_str(),
        );
        records.append(&mut derived);
    }
    Ok(ConsequenceEventPlan {
        world_id: current.world_id.clone(),
        turn_id: current.turn_id.clone(),
        records,
    })
}

pub fn audit_consequence_contract(
    context: &PromptContextPacket,
    current: &ConsequenceSpinePacket,
    proposal: Option<&ConsequenceProposal>,
    resolution: Option<&ResolutionProposal>,
) -> Result<()> {
    if let Some(proposal) = proposal {
        validate_consequence_proposal(current, proposal)?;
        let visible_refs =
            collect_visible_strings(&serde_json::to_value(&context.visible_context)?);
        for mutation in proposal.introduced.iter().chain(proposal.updated.iter()) {
            require_visible_refs(&visible_refs, "consequence mutation", &mutation.source_refs)?;
            if matches!(
                mutation.severity,
                ConsequenceSeverity::Major | ConsequenceSeverity::Critical
            ) && mutation.linked_projection_refs.is_empty()
            {
                bail!("major/critical consequence mutation requires linked_projection_refs");
            }
            reject_hidden_text(context, mutation.summary.as_str())?;
            reject_hidden_text(context, mutation.player_visible_signal.as_str())?;
        }
        for payoff in &proposal.paid_off {
            require_visible_refs(&visible_refs, "consequence payoff", &payoff.evidence_refs)?;
            reject_hidden_text(context, payoff.payoff_summary.as_str())?;
        }
        for ephemeral in &proposal.ephemeral_effects {
            require_visible_refs(
                &visible_refs,
                "ephemeral consequence reason",
                &ephemeral.evidence_refs,
            )?;
            reject_hidden_text(context, ephemeral.reason.as_str())?;
        }
    }
    if let Some(resolution) = resolution {
        audit_significant_resolution_has_consequence_path(proposal, resolution)?;
    }
    Ok(())
}

fn audit_significant_resolution_has_consequence_path(
    proposal: Option<&ConsequenceProposal>,
    resolution: &ResolutionProposal,
) -> Result<()> {
    let covered_refs = proposal.map(covered_consequence_refs).unwrap_or_default();
    for effect in &resolution.proposed_effects {
        if effect.visibility != ResolutionVisibility::PlayerVisible
            || !is_significant_effect(effect.effect_kind)
        {
            continue;
        }
        if !effect
            .evidence_refs
            .iter()
            .any(|reference| covered_refs.contains(reference.as_str()))
            && consequence_for_effect(effect.effect_kind).is_none()
        {
            bail!(
                "significant resolution effect has no consequence path: {}",
                effect.target_ref
            );
        }
    }
    Ok(())
}

fn covered_consequence_refs(proposal: &ConsequenceProposal) -> BTreeSet<&str> {
    proposal
        .introduced
        .iter()
        .chain(proposal.updated.iter())
        .flat_map(|mutation| mutation.source_refs.iter().map(String::as_str))
        .chain(
            proposal
                .paid_off
                .iter()
                .flat_map(|payoff| payoff.evidence_refs.iter().map(String::as_str)),
        )
        .chain(
            proposal
                .ephemeral_effects
                .iter()
                .flat_map(|ephemeral| ephemeral.evidence_refs.iter().map(String::as_str)),
        )
        .collect()
}

const fn is_significant_effect(effect_kind: ProposedEffectKind) -> bool {
    matches!(
        effect_kind,
        ProposedEffectKind::BodyResourceDelta
            | ProposedEffectKind::LocationDelta
            | ProposedEffectKind::RelationshipDelta
            | ProposedEffectKind::BeliefDelta
            | ProposedEffectKind::WorldLoreDelta
            | ProposedEffectKind::ScenePressureDelta
    )
}

fn require_visible_refs(
    visible_refs: &BTreeSet<String>,
    label: &str,
    refs: &[String],
) -> Result<()> {
    if refs.is_empty() {
        bail!("{label} requires evidence/source refs");
    }
    let missing = refs
        .iter()
        .filter(|reference| !visible_refs.contains(reference.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!(
            "{label} references non-visible refs: {}",
            missing.join(", ")
        );
    }
    Ok(())
}

fn reject_hidden_text(context: &PromptContextPacket, text: &str) -> Result<()> {
    if text.trim().is_empty() {
        return Ok(());
    }
    let hidden = serde_json::to_value(&context.adjudication_context)?;
    if collect_visible_strings(&hidden)
        .into_iter()
        .filter(|hidden_text| hidden_text.chars().count() >= 8)
        .any(|hidden_text| text.contains(hidden_text.as_str()))
    {
        bail!("consequence visible field contains hidden/adjudication-only text");
    }
    Ok(())
}

fn collect_visible_strings(value: &serde_json::Value) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    collect_visible_strings_into(value, &mut refs);
    refs
}

fn collect_visible_strings_into(value: &serde_json::Value, refs: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::String(text) if !text.trim().is_empty() => {
            refs.insert(text.to_owned());
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_visible_strings_into(item, refs);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                collect_visible_strings_into(item, refs);
            }
        }
        _ => {}
    }
}

fn validate_consequence_proposal(
    current: &ConsequenceSpinePacket,
    proposal: &ConsequenceProposal,
) -> Result<()> {
    if proposal.schema_version != CONSEQUENCE_PROPOSAL_SCHEMA_VERSION {
        bail!("consequence proposal schema_version mismatch");
    }
    if proposal.world_id != current.world_id || proposal.turn_id != current.turn_id {
        bail!("consequence proposal world_id/turn_id mismatch");
    }
    for mutation in proposal.introduced.iter().chain(proposal.updated.iter()) {
        if mutation.source_refs.is_empty() {
            bail!("consequence mutation requires source_refs");
        }
        if mutation.summary.trim().is_empty() || mutation.player_visible_signal.trim().is_empty() {
            bail!("consequence mutation requires player-visible summary and signal");
        }
    }
    for ephemeral in &proposal.ephemeral_effects {
        if ephemeral.evidence_refs.is_empty() {
            bail!("ephemeral consequence reason requires evidence_refs");
        }
    }
    Ok(())
}

fn record_from_mutation(
    current: &ConsequenceSpinePacket,
    mutation: &ConsequenceMutation,
    event_kind: ConsequenceEventKind,
    index: usize,
    recorded_at: &str,
) -> ConsequenceEventRecord {
    ConsequenceEventRecord {
        schema_version: CONSEQUENCE_EVENT_SCHEMA_VERSION.to_owned(),
        world_id: current.world_id.clone(),
        turn_id: current.turn_id.clone(),
        event_id: format!("consequence_event:{}:{index:02}", current.turn_id),
        consequence_id: mutation.consequence_id.clone(),
        event_kind,
        kind: mutation.kind,
        scope: mutation.scope,
        severity: mutation.severity,
        summary: mutation.summary.clone(),
        player_visible_signal: mutation.player_visible_signal.clone(),
        source_refs: mutation.source_refs.clone(),
        linked_entity_refs: mutation.linked_entity_refs.clone(),
        linked_projection_refs: mutation.linked_projection_refs.clone(),
        recorded_at: recorded_at.to_owned(),
    }
}

fn derive_consequence_records(
    current: &ConsequenceSpinePacket,
    resolution: &ResolutionProposal,
    offset: usize,
    existing_refs: &BTreeSet<&str>,
    recorded_at: &str,
) -> Vec<ConsequenceEventRecord> {
    let mut records = Vec::new();
    for (index, gate) in resolution.gate_results.iter().enumerate() {
        if gate.visibility != ResolutionVisibility::PlayerVisible
            || gate.status != GateStatus::CostImposed
            || gate
                .evidence_refs
                .iter()
                .any(|evidence| existing_refs.contains(evidence.as_str()))
        {
            continue;
        }
        let (kind, scope) = consequence_for_gate(gate.gate_kind);
        records.push(derived_record(
            current,
            resolution,
            offset + records.len(),
            kind,
            scope,
            ConsequenceSeverity::Moderate,
            format!("consequence:{}:gate:{index}", resolution.turn_id),
            gate.reason.clone(),
            "그 선택의 비용이 아직 장면에 남아 있다.".to_owned(),
            gate.evidence_refs.clone(),
            vec![gate.gate_ref.clone()],
            recorded_at,
        ));
    }
    for (index, effect) in resolution.proposed_effects.iter().enumerate() {
        if effect.visibility != ResolutionVisibility::PlayerVisible
            || effect
                .evidence_refs
                .iter()
                .any(|evidence| existing_refs.contains(evidence.as_str()))
        {
            continue;
        }
        let Some((kind, scope, severity)) = consequence_for_effect(effect.effect_kind) else {
            continue;
        };
        records.push(derived_record(
            current,
            resolution,
            offset + records.len(),
            kind,
            scope,
            severity,
            format!("consequence:{}:effect:{index}", resolution.turn_id),
            effect.summary.clone(),
            signal_for_kind(kind),
            effect.evidence_refs.clone(),
            vec![effect.target_ref.clone()],
            recorded_at,
        ));
    }
    for (index, tick) in resolution.process_ticks.iter().enumerate() {
        if tick.visibility != ResolutionVisibility::PlayerVisible {
            continue;
        }
        records.push(derived_record(
            current,
            resolution,
            offset + records.len(),
            ConsequenceKind::ProcessAccelerated,
            ConsequenceScope::WorldProcess,
            ConsequenceSeverity::Moderate,
            format!("consequence:{}:process:{index}", resolution.turn_id),
            tick.summary.clone(),
            "건드린 진행 시계가 다음 장면에도 압력으로 남는다.".to_owned(),
            tick.evidence_refs.clone(),
            vec![tick.process_ref.clone()],
            recorded_at,
        ));
    }
    if resolution.outcome.kind == ResolutionOutcomeKind::CostlySuccess
        && records.is_empty()
        && !resolution.outcome.evidence_refs.is_empty()
    {
        records.push(derived_record(
            current,
            resolution,
            offset,
            ConsequenceKind::MoralDebt,
            ConsequenceScope::Scene,
            ConsequenceSeverity::Moderate,
            format!("consequence:{}:costly_success", resolution.turn_id),
            resolution.outcome.summary.clone(),
            "성공의 대가가 아직 완전히 사라지지 않았다.".to_owned(),
            resolution.outcome.evidence_refs.clone(),
            Vec::new(),
            recorded_at,
        ));
    }
    records
}

fn derive_consequence_records_from_social_exchange(
    current: &ConsequenceSpinePacket,
    social_exchange: &SocialExchangeProposal,
    offset: usize,
    existing_refs: &BTreeSet<&str>,
    recorded_at: &str,
) -> Vec<ConsequenceEventRecord> {
    let mut records = Vec::new();
    for (index, exchange) in social_exchange.exchanges.iter().enumerate() {
        if exchange
            .source_refs
            .iter()
            .any(|reference| existing_refs.contains(reference.as_str()))
            || !exchange_should_be_consequence(exchange.stance_after, exchange.intensity_after)
        {
            continue;
        }
        records.push(derived_social_record(
            current,
            offset + records.len(),
            consequence_kind_from_stance(exchange.stance_after),
            ConsequenceScope::Relationship,
            severity_from_social_intensity(exchange.intensity_after),
            format!(
                "consequence:{}:social_exchange:{index}",
                social_exchange.turn_id
            ),
            exchange.summary.clone(),
            exchange.player_visible_signal.clone(),
            exchange.source_refs.clone(),
            exchange
                .relationship_refs
                .iter()
                .chain(exchange.consequence_refs.iter())
                .cloned()
                .collect(),
            recorded_at,
        ));
    }
    for (index, commitment) in social_exchange.commitments.iter().enumerate() {
        if commitment
            .source_refs
            .iter()
            .any(|reference| existing_refs.contains(reference.as_str()))
        {
            continue;
        }
        records.push(derived_social_record(
            current,
            offset + records.len(),
            consequence_kind_from_commitment(commitment.kind),
            ConsequenceScope::Relationship,
            ConsequenceSeverity::Moderate,
            format!(
                "consequence:{}:social_commitment:{index}",
                social_exchange.turn_id
            ),
            commitment.summary.clone(),
            commitment.condition.clone(),
            commitment.source_refs.clone(),
            commitment.consequence_refs.clone(),
            recorded_at,
        ));
    }
    records
}

#[allow(clippy::too_many_arguments)]
fn derived_social_record(
    current: &ConsequenceSpinePacket,
    index: usize,
    kind: ConsequenceKind,
    scope: ConsequenceScope,
    severity: ConsequenceSeverity,
    consequence_id: String,
    summary: String,
    player_visible_signal: String,
    source_refs: Vec<String>,
    linked_projection_refs: Vec<String>,
    recorded_at: &str,
) -> ConsequenceEventRecord {
    ConsequenceEventRecord {
        schema_version: CONSEQUENCE_EVENT_SCHEMA_VERSION.to_owned(),
        world_id: current.world_id.clone(),
        turn_id: current.turn_id.clone(),
        event_id: format!("consequence_event:{}:{index:02}", current.turn_id),
        consequence_id,
        event_kind: ConsequenceEventKind::Introduced,
        kind,
        scope,
        severity,
        summary,
        player_visible_signal,
        source_refs,
        linked_entity_refs: Vec::new(),
        linked_projection_refs,
        recorded_at: recorded_at.to_owned(),
    }
}

fn exchange_should_be_consequence(stance: DialogueStanceKind, intensity: SocialIntensity) -> bool {
    matches!(intensity, SocialIntensity::High | SocialIntensity::Crisis)
        || matches!(
            stance,
            DialogueStanceKind::Offended
                | DialogueStanceKind::Threatening
                | DialogueStanceKind::Indebted
                | DialogueStanceKind::Withholding
        )
}

const fn consequence_kind_from_stance(stance: DialogueStanceKind) -> ConsequenceKind {
    match stance {
        DialogueStanceKind::Offended | DialogueStanceKind::Threatening => {
            ConsequenceKind::SuspicionRaised
        }
        DialogueStanceKind::Withholding | DialogueStanceKind::Evasive => {
            ConsequenceKind::OpportunityLost
        }
        DialogueStanceKind::Cooperative | DialogueStanceKind::GuardedHelpful => {
            ConsequenceKind::TrustShift
        }
        _ => ConsequenceKind::SocialDebt,
    }
}

const fn consequence_kind_from_commitment(kind: SocialCommitmentKind) -> ConsequenceKind {
    match kind {
        SocialCommitmentKind::Promise
        | SocialCommitmentKind::Debt
        | SocialCommitmentKind::ConditionalPermission
        | SocialCommitmentKind::Truce => ConsequenceKind::SocialDebt,
        SocialCommitmentKind::Threat | SocialCommitmentKind::Demand => {
            ConsequenceKind::SuspicionRaised
        }
        SocialCommitmentKind::Price => ConsequenceKind::OpportunityOpened,
    }
}

const fn severity_from_social_intensity(intensity: SocialIntensity) -> ConsequenceSeverity {
    match intensity {
        SocialIntensity::Trace | SocialIntensity::Low => ConsequenceSeverity::Minor,
        SocialIntensity::Medium => ConsequenceSeverity::Moderate,
        SocialIntensity::High => ConsequenceSeverity::Major,
        SocialIntensity::Crisis => ConsequenceSeverity::Critical,
    }
}

#[allow(clippy::too_many_arguments)]
fn derived_record(
    current: &ConsequenceSpinePacket,
    resolution: &ResolutionProposal,
    index: usize,
    kind: ConsequenceKind,
    scope: ConsequenceScope,
    severity: ConsequenceSeverity,
    consequence_id: String,
    summary: String,
    player_visible_signal: String,
    source_refs: Vec<String>,
    linked_projection_refs: Vec<String>,
    recorded_at: &str,
) -> ConsequenceEventRecord {
    ConsequenceEventRecord {
        schema_version: CONSEQUENCE_EVENT_SCHEMA_VERSION.to_owned(),
        world_id: current.world_id.clone(),
        turn_id: current.turn_id.clone(),
        event_id: format!("consequence_event:{}:{index:02}", resolution.turn_id),
        consequence_id,
        event_kind: ConsequenceEventKind::Introduced,
        kind,
        scope,
        severity,
        summary,
        player_visible_signal,
        source_refs,
        linked_entity_refs: Vec::new(),
        linked_projection_refs,
        recorded_at: recorded_at.to_owned(),
    }
}

const fn consequence_for_gate(gate_kind: GateKind) -> (ConsequenceKind, ConsequenceScope) {
    match gate_kind {
        GateKind::Body => (ConsequenceKind::BodyCost, ConsequenceScope::Body),
        GateKind::Resource => (ConsequenceKind::ResourceCost, ConsequenceScope::Inventory),
        GateKind::Location => (
            ConsequenceKind::LocationAccessChanged,
            ConsequenceScope::Location,
        ),
        GateKind::SocialPermission => (
            ConsequenceKind::SuspicionRaised,
            ConsequenceScope::Relationship,
        ),
        GateKind::Knowledge => (
            ConsequenceKind::KnowledgeOpened,
            ConsequenceScope::Knowledge,
        ),
        GateKind::TimePressure | GateKind::HiddenConstraint => (
            ConsequenceKind::ProcessAccelerated,
            ConsequenceScope::WorldProcess,
        ),
        GateKind::WorldLaw | GateKind::Affordance => {
            (ConsequenceKind::OpportunityLost, ConsequenceScope::Scene)
        }
    }
}

const fn consequence_for_effect(
    effect_kind: ProposedEffectKind,
) -> Option<(ConsequenceKind, ConsequenceScope, ConsequenceSeverity)> {
    match effect_kind {
        ProposedEffectKind::BodyResourceDelta => Some((
            ConsequenceKind::BodyCost,
            ConsequenceScope::Body,
            ConsequenceSeverity::Moderate,
        )),
        ProposedEffectKind::LocationDelta => Some((
            ConsequenceKind::LocationAccessChanged,
            ConsequenceScope::Location,
            ConsequenceSeverity::Moderate,
        )),
        ProposedEffectKind::RelationshipDelta => Some((
            ConsequenceKind::TrustShift,
            ConsequenceScope::Relationship,
            ConsequenceSeverity::Moderate,
        )),
        ProposedEffectKind::BeliefDelta | ProposedEffectKind::WorldLoreDelta => Some((
            ConsequenceKind::KnowledgeOpened,
            ConsequenceScope::Knowledge,
            ConsequenceSeverity::Minor,
        )),
        ProposedEffectKind::ScenePressureDelta => Some((
            ConsequenceKind::OpportunityOpened,
            ConsequenceScope::Scene,
            ConsequenceSeverity::Minor,
        )),
        ProposedEffectKind::PatternDebt | ProposedEffectKind::PlayerIntentTrace => None,
    }
}

fn signal_for_kind(kind: ConsequenceKind) -> String {
    match kind {
        ConsequenceKind::BodyCost => "몸에 남은 대가가 선택을 좁힌다.",
        ConsequenceKind::ResourceCost => "쓴 자원이나 잃은 물건이 다음 판단에 남는다.",
        ConsequenceKind::SocialDebt | ConsequenceKind::TrustShift => {
            "관계의 변화가 다음 대화의 출발점을 바꾼다."
        }
        ConsequenceKind::SuspicionRaised | ConsequenceKind::AlarmRaised => {
            "의심이나 경계가 아직 가라앉지 않았다."
        }
        ConsequenceKind::KnowledgeOpened | ConsequenceKind::KnowledgeResolved => {
            "알게 된 사실이 다음 행동의 방향을 바꾼다."
        }
        ConsequenceKind::LocationAccessChanged => "장소 접근 조건이 달라졌다.",
        ConsequenceKind::ProcessAccelerated | ConsequenceKind::ProcessDelayed => {
            "세계 진행 시계가 선택의 여파를 들고 움직인다."
        }
        ConsequenceKind::MoralDebt => "선택의 도덕적 빚이 장면에 남아 있다.",
        ConsequenceKind::OpportunityOpened | ConsequenceKind::OpportunityLost => {
            "열리거나 닫힌 기회가 다음 선택지를 바꾼다."
        }
    }
    .to_owned()
}

pub fn append_consequence_event_plan(world_dir: &Path, plan: &ConsequenceEventPlan) -> Result<()> {
    for record in &plan.records {
        append_jsonl(&world_dir.join(CONSEQUENCE_EVENTS_FILENAME), record)?;
    }
    Ok(())
}

pub fn rebuild_consequence_spine(
    world_dir: &Path,
    base_packet: &ConsequenceSpinePacket,
) -> Result<ConsequenceSpinePacket> {
    let records = load_consequence_event_records(world_dir)?;
    let mut active_by_id: BTreeMap<String, ActiveConsequence> = BTreeMap::new();
    let mut paid_off = Vec::new();
    for record in records {
        match record.event_kind {
            ConsequenceEventKind::Introduced
            | ConsequenceEventKind::Escalated
            | ConsequenceEventKind::Softened
            | ConsequenceEventKind::Transferred => {
                active_by_id.insert(record.consequence_id.clone(), active_from_record(record));
            }
            ConsequenceEventKind::PaidOff
            | ConsequenceEventKind::Decayed
            | ConsequenceEventKind::Superseded => {
                active_by_id.remove(&record.consequence_id);
                if record.event_kind == ConsequenceEventKind::PaidOff {
                    paid_off.push(ConsequenceMemory {
                        consequence_id: record.consequence_id,
                        kind: record.kind,
                        summary: record.player_visible_signal.clone(),
                        payoff_summary: record.summary,
                        source_refs: record.source_refs,
                    });
                }
            }
        }
    }
    let mut active = active_by_id.into_values().rev().collect::<Vec<_>>();
    active.truncate(ACTIVE_CONSEQUENCE_BUDGET);
    paid_off.reverse();
    paid_off.truncate(PAID_OFF_MEMORY_BUDGET);
    let pressure_links = active.iter().map(pressure_link_for).collect();
    let packet = ConsequenceSpinePacket {
        schema_version: CONSEQUENCE_SPINE_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: base_packet.world_id.clone(),
        turn_id: base_packet.turn_id.clone(),
        active,
        recently_paid_off: paid_off,
        pressure_links,
        required_followups: Vec::new(),
        compiler_policy: ConsequenceSpinePolicy::default(),
    };
    write_json(&world_dir.join(ACTIVE_CONSEQUENCES_FILENAME), &packet)?;
    Ok(packet)
}

pub fn load_consequence_spine_state(
    world_dir: &Path,
    fallback: ConsequenceSpinePacket,
) -> Result<ConsequenceSpinePacket> {
    let path = world_dir.join(ACTIVE_CONSEQUENCES_FILENAME);
    if path.is_file() {
        return read_json(&path);
    }
    Ok(fallback)
}

fn load_consequence_event_records(world_dir: &Path) -> Result<Vec<ConsequenceEventRecord>> {
    let path = world_dir.join(CONSEQUENCE_EVENTS_FILENAME);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str::<ConsequenceEventRecord>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn active_from_record(record: ConsequenceEventRecord) -> ActiveConsequence {
    ActiveConsequence {
        schema_version: CONSEQUENCE_SCHEMA_VERSION.to_owned(),
        consequence_id: record.consequence_id,
        origin_turn_id: record.turn_id,
        kind: record.kind,
        scope: record.scope,
        status: status_from_event(record.event_kind),
        severity: record.severity,
        summary: record.summary,
        player_visible_signal: record.player_visible_signal,
        source_refs: record.source_refs,
        linked_entity_refs: record.linked_entity_refs,
        linked_projection_refs: record.linked_projection_refs,
        expected_return: return_window_for_severity(record.severity),
        decay: ConsequenceDecay::default(),
    }
}

const fn status_from_event(event_kind: ConsequenceEventKind) -> ConsequenceStatus {
    match event_kind {
        ConsequenceEventKind::Introduced => ConsequenceStatus::Active,
        ConsequenceEventKind::Escalated => ConsequenceStatus::Escalated,
        ConsequenceEventKind::Softened => ConsequenceStatus::Softened,
        ConsequenceEventKind::Transferred => ConsequenceStatus::Transferred,
        ConsequenceEventKind::PaidOff => ConsequenceStatus::PaidOff,
        ConsequenceEventKind::Decayed => ConsequenceStatus::Decayed,
        ConsequenceEventKind::Superseded => ConsequenceStatus::Superseded,
    }
}

const fn return_window_for_severity(severity: ConsequenceSeverity) -> ConsequenceReturnWindow {
    match severity {
        ConsequenceSeverity::Trace => ConsequenceReturnWindow::SearchOnly,
        ConsequenceSeverity::Minor => ConsequenceReturnWindow::WhenRelevant,
        ConsequenceSeverity::Moderate => ConsequenceReturnWindow::Soon,
        ConsequenceSeverity::Major => ConsequenceReturnWindow::CurrentScene,
        ConsequenceSeverity::Critical => ConsequenceReturnWindow::NextTurn,
    }
}

fn pressure_link_for(consequence: &ActiveConsequence) -> ConsequencePressureLink {
    ConsequencePressureLink {
        consequence_id: consequence.consequence_id.clone(),
        pressure_kind: match consequence.kind {
            ConsequenceKind::BodyCost => "body",
            ConsequenceKind::ResourceCost => "resource",
            ConsequenceKind::SocialDebt
            | ConsequenceKind::TrustShift
            | ConsequenceKind::SuspicionRaised => "social_permission",
            ConsequenceKind::AlarmRaised => "threat",
            ConsequenceKind::KnowledgeOpened | ConsequenceKind::KnowledgeResolved => "knowledge",
            ConsequenceKind::LocationAccessChanged => "environment",
            ConsequenceKind::ProcessAccelerated | ConsequenceKind::ProcessDelayed => {
                "time_pressure"
            }
            ConsequenceKind::MoralDebt => "moral_cost",
            ConsequenceKind::OpportunityOpened | ConsequenceKind::OpportunityLost => "desire",
        }
        .to_owned(),
        observable_signal: consequence.player_visible_signal.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolution::{
        ActionAmbiguity, ActionInputKind, ActionIntent, ChoicePlan, NarrativeBrief,
        RESOLUTION_PROPOSAL_SCHEMA_VERSION, ResolutionOutcome,
    };

    #[test]
    fn derives_consequence_from_costly_resolution_effect() -> anyhow::Result<()> {
        let packet = ConsequenceSpinePacket {
            world_id: "world".to_owned(),
            turn_id: "turn_0003".to_owned(),
            ..ConsequenceSpinePacket::default()
        };
        let proposal = sample_resolution();
        let plan = prepare_consequence_event_plan(&packet, None, Some(&proposal), None)?;

        assert!(
            plan.records
                .iter()
                .any(|record| record.kind == ConsequenceKind::SuspicionRaised)
        );
        assert!(
            plan.records
                .iter()
                .any(|record| record.kind == ConsequenceKind::TrustShift)
        );
        Ok(())
    }

    #[test]
    fn materializes_active_and_paid_off_consequences() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let packet = ConsequenceSpinePacket {
            world_id: "world".to_owned(),
            turn_id: "turn_0003".to_owned(),
            ..ConsequenceSpinePacket::default()
        };
        let plan = prepare_consequence_event_plan(&packet, None, Some(&sample_resolution()), None)?;
        append_consequence_event_plan(temp.path(), &plan)?;
        let rebuilt = rebuild_consequence_spine(temp.path(), &packet)?;

        assert!(!rebuilt.active.is_empty());
        assert!(!rebuilt.pressure_links.is_empty());
        assert!(temp.path().join(ACTIVE_CONSEQUENCES_FILENAME).is_file());
        Ok(())
    }

    #[test]
    fn derives_consequence_from_social_exchange_commitment() -> anyhow::Result<()> {
        let packet = ConsequenceSpinePacket {
            world_id: "world".to_owned(),
            turn_id: "turn_0003".to_owned(),
            ..ConsequenceSpinePacket::default()
        };
        let social = SocialExchangeProposal {
            schema_version: crate::social_exchange::SOCIAL_EXCHANGE_PROPOSAL_SCHEMA_VERSION
                .to_owned(),
            world_id: "world".to_owned(),
            turn_id: "turn_0003".to_owned(),
            exchanges: Vec::new(),
            commitments: vec![crate::social_exchange::SocialCommitmentMutation {
                commitment_id: "commitment:guard:entry".to_owned(),
                actor_ref: "char:guard".to_owned(),
                target_ref: "player".to_owned(),
                kind: SocialCommitmentKind::ConditionalPermission,
                summary: "문지기는 신원을 밝히면 검문을 이어가겠다고 했다.".to_owned(),
                condition: "이름과 온 길을 말해야 한다.".to_owned(),
                due_window: crate::social_exchange::SocialDueWindow::CurrentScene,
                source_refs: vec!["visible_scene.text_blocks[0]".to_owned()],
                consequence_refs: Vec::new(),
            }],
            unresolved_asks: Vec::new(),
            leverage_updates: Vec::new(),
            paid_off_or_closed: Vec::new(),
            ephemeral_social_notes: Vec::new(),
        };
        let plan = prepare_consequence_event_plan(&packet, None, None, Some(&social))?;

        assert_eq!(plan.records.len(), 1);
        assert_eq!(plan.records[0].kind, ConsequenceKind::SocialDebt);
        Ok(())
    }

    fn sample_resolution() -> ResolutionProposal {
        ResolutionProposal {
            schema_version: RESOLUTION_PROPOSAL_SCHEMA_VERSION.to_owned(),
            world_id: "world".to_owned(),
            turn_id: "turn_0003".to_owned(),
            interpreted_intent: ActionIntent {
                input_kind: ActionInputKind::PresentedChoice,
                summary: "문지기를 밀치고 들어간다".to_owned(),
                target_refs: vec!["choice:1".to_owned()],
                pressure_refs: Vec::new(),
                evidence_refs: vec!["choice:1".to_owned()],
                ambiguity: ActionAmbiguity::Clear,
            },
            outcome: ResolutionOutcome {
                kind: ResolutionOutcomeKind::CostlySuccess,
                summary: "안으로 들어가지만 문지기의 의심을 산다.".to_owned(),
                evidence_refs: vec!["choice:1".to_owned()],
            },
            gate_results: vec![crate::resolution::GateResult {
                gate_kind: GateKind::SocialPermission,
                gate_ref: "rel:guard".to_owned(),
                visibility: ResolutionVisibility::PlayerVisible,
                status: GateStatus::CostImposed,
                reason: "절차를 무시한 행동이 의심을 남긴다.".to_owned(),
                evidence_refs: vec!["choice:1".to_owned()],
            }],
            proposed_effects: vec![crate::resolution::ProposedEffect {
                effect_kind: ProposedEffectKind::RelationshipDelta,
                target_ref: "rel:guard".to_owned(),
                visibility: ResolutionVisibility::PlayerVisible,
                summary: "문지기는 주인공을 절차를 무시한 사람으로 기억한다.".to_owned(),
                evidence_refs: vec!["choice:1".to_owned()],
            }],
            process_ticks: Vec::new(),
            narrative_brief: NarrativeBrief {
                visible_summary: "의심이 남는다.".to_owned(),
                required_beats: Vec::new(),
                forbidden_visible_details: Vec::new(),
            },
            next_choice_plan: Vec::<ChoicePlan>::new(),
        }
    }
}
