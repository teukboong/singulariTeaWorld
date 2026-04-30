#![allow(clippy::missing_errors_doc)]

use crate::agent_bridge::AgentTurnResponse;
use crate::consequence_spine::{ActiveConsequence, ConsequenceKind, ConsequenceSpinePacket};
use crate::prompt_context::PromptContextPacket;
use crate::resolution::{
    GateKind, GateStatus, ResolutionOutcomeKind, ResolutionProposal, ResolutionVisibility,
};
use crate::response_context::AgentRelationshipUpdate;
use crate::store::{append_jsonl, read_json, write_json};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const SOCIAL_EXCHANGE_PACKET_SCHEMA_VERSION: &str = "singulari.social_exchange_packet.v1";
pub const SOCIAL_EXCHANGE_EVENT_SCHEMA_VERSION: &str = "singulari.social_exchange_event.v1";
pub const SOCIAL_EXCHANGE_PROPOSAL_SCHEMA_VERSION: &str = "singulari.social_exchange_proposal.v1";
pub const SOCIAL_EXCHANGE_STANCE_SCHEMA_VERSION: &str = "singulari.dialogue_stance.v1";
pub const SOCIAL_COMMITMENT_SCHEMA_VERSION: &str = "singulari.social_commitment.v1";
pub const UNRESOLVED_SOCIAL_ASK_SCHEMA_VERSION: &str = "singulari.unresolved_social_ask.v1";
pub const CONVERSATION_LEVERAGE_SCHEMA_VERSION: &str = "singulari.conversation_leverage.v1";
pub const SOCIAL_EXCHANGE_EVENTS_FILENAME: &str = "social_exchange_events.jsonl";
pub const DIALOGUE_STANCE_FILENAME: &str = "dialogue_stance.json";

const STANCE_BUDGET: usize = 8;
const COMMITMENT_BUDGET: usize = 8;
const ASK_BUDGET: usize = 8;
const EXCHANGE_MEMORY_BUDGET: usize = 10;
const LEVERAGE_BUDGET: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SocialExchangePacket {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub active_stances: Vec<DialogueStance>,
    #[serde(default)]
    pub active_commitments: Vec<SocialCommitment>,
    #[serde(default)]
    pub unresolved_asks: Vec<UnresolvedSocialAsk>,
    #[serde(default)]
    pub recent_exchanges: Vec<SocialExchangeMemory>,
    #[serde(default)]
    pub leverage: Vec<ConversationLeverage>,
    pub compiler_policy: SocialExchangePolicy,
}

impl Default for SocialExchangePacket {
    fn default() -> Self {
        Self {
            schema_version: SOCIAL_EXCHANGE_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            turn_id: String::new(),
            active_stances: Vec::new(),
            active_commitments: Vec::new(),
            unresolved_asks: Vec::new(),
            recent_exchanges: Vec::new(),
            leverage: Vec::new(),
            compiler_policy: SocialExchangePolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SocialExchangePolicy {
    pub source: String,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for SocialExchangePolicy {
    fn default() -> Self {
        Self {
            source: "materialized_from_social_exchange_events_v1".to_owned(),
            use_rules: vec![
                "Use active stances as current dialogue posture, not as durable relationship state.".to_owned(),
                "Use unresolved asks to avoid repeating the same question without new leverage.".to_owned(),
                "Player-visible summaries must not reveal hidden motives or adjudication-only facts.".to_owned(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DialogueStance {
    pub schema_version: String,
    pub stance_id: String,
    pub actor_ref: String,
    pub target_ref: String,
    pub stance: DialogueStanceKind,
    pub intensity: SocialIntensity,
    pub summary: String,
    pub player_visible_signal: String,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default)]
    pub relationship_refs: Vec<String>,
    #[serde(default)]
    pub consequence_refs: Vec<String>,
    pub opened_turn_id: String,
    pub last_changed_turn_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DialogueStanceKind {
    NeutralProcedure,
    WaryTesting,
    Cooperative,
    GuardedHelpful,
    Offended,
    Evasive,
    Threatening,
    Bargaining,
    Indebted,
    Pressuring,
    Appeasing,
    Withholding,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SocialIntensity {
    Trace,
    Low,
    Medium,
    High,
    Crisis,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SocialExchangeActKind {
    Ask,
    Answer,
    Evade,
    Refuse,
    Offer,
    Accept,
    CounterOffer,
    Threaten,
    Apologize,
    Insult,
    Promise,
    Demand,
    RevealConditionally,
    Withhold,
    Test,
    GrantPermission,
    RevokePermission,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SocialCommitment {
    pub schema_version: String,
    pub commitment_id: String,
    pub actor_ref: String,
    pub target_ref: String,
    pub kind: SocialCommitmentKind,
    pub status: SocialCommitmentStatus,
    pub summary: String,
    pub condition: String,
    pub due_window: SocialDueWindow,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default)]
    pub consequence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SocialCommitmentKind {
    Promise,
    Debt,
    Demand,
    ConditionalPermission,
    Truce,
    Threat,
    Price,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SocialCommitmentStatus {
    Active,
    Fulfilled,
    Violated,
    Waived,
    Expired,
    Superseded,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SocialDueWindow {
    Immediate,
    CurrentScene,
    Soon,
    WhenRelevant,
    SearchOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnresolvedSocialAsk {
    pub schema_version: String,
    pub ask_id: String,
    pub asked_by_ref: String,
    pub asked_to_ref: String,
    pub question_summary: String,
    pub current_status: AskStatus,
    pub last_response: String,
    #[serde(default)]
    pub allowed_next_moves: Vec<String>,
    #[serde(default)]
    pub blocked_repetitions: Vec<String>,
    #[serde(default)]
    pub source_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AskStatus {
    Open,
    Answered,
    Evaded,
    Refused,
    Conditional,
    Obsolete,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConversationLeverage {
    pub schema_version: String,
    pub leverage_id: String,
    pub holder_ref: String,
    pub target_ref: String,
    pub leverage_kind: ConversationLeverageKind,
    pub summary: String,
    pub expires: SocialDueWindow,
    #[serde(default)]
    pub source_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationLeverageKind {
    HasInformation,
    ControlsAccess,
    OwesFavor,
    CanEmbarrass,
    CanThreaten,
    NeedsHelp,
    HasWitnesses,
    HoldsResource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SocialExchangeMemory {
    pub event_id: String,
    pub actor_ref: String,
    pub target_ref: String,
    pub act_kind: SocialExchangeActKind,
    pub summary: String,
    pub player_visible_signal: String,
    #[serde(default)]
    pub source_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SocialExchangeProposal {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub exchanges: Vec<SocialExchangeMutation>,
    #[serde(default)]
    pub commitments: Vec<SocialCommitmentMutation>,
    #[serde(default)]
    pub unresolved_asks: Vec<UnresolvedAskMutation>,
    #[serde(default)]
    pub leverage_updates: Vec<ConversationLeverageMutation>,
    #[serde(default)]
    pub paid_off_or_closed: Vec<SocialExchangeClosure>,
    #[serde(default)]
    pub ephemeral_social_notes: Vec<EphemeralSocialNote>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SocialExchangeMutation {
    pub actor_ref: String,
    pub target_ref: String,
    pub act_kind: SocialExchangeActKind,
    pub stance_after: DialogueStanceKind,
    pub intensity_after: SocialIntensity,
    pub summary: String,
    pub player_visible_signal: String,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default)]
    pub relationship_refs: Vec<String>,
    #[serde(default)]
    pub consequence_refs: Vec<String>,
    #[serde(default)]
    pub commitment_refs: Vec<String>,
    #[serde(default)]
    pub unresolved_ask_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SocialCommitmentMutation {
    pub commitment_id: String,
    pub actor_ref: String,
    pub target_ref: String,
    pub kind: SocialCommitmentKind,
    pub summary: String,
    pub condition: String,
    pub due_window: SocialDueWindow,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default)]
    pub consequence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnresolvedAskMutation {
    pub ask_id: String,
    pub asked_by_ref: String,
    pub asked_to_ref: String,
    pub question_summary: String,
    pub current_status: AskStatus,
    pub last_response: String,
    #[serde(default)]
    pub allowed_next_moves: Vec<String>,
    #[serde(default)]
    pub blocked_repetitions: Vec<String>,
    #[serde(default)]
    pub source_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConversationLeverageMutation {
    pub leverage_id: String,
    pub holder_ref: String,
    pub target_ref: String,
    pub leverage_kind: ConversationLeverageKind,
    pub summary: String,
    pub expires: SocialDueWindow,
    #[serde(default)]
    pub source_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SocialExchangeClosure {
    pub ref_id: String,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EphemeralSocialNote {
    pub note: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SocialExchangeEventPlan {
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub records: Vec<SocialExchangeEventRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SocialExchangeEventRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub event_id: String,
    pub actor_ref: String,
    pub target_ref: String,
    pub act_kind: SocialExchangeActKind,
    pub stance_after: DialogueStanceKind,
    pub intensity_after: SocialIntensity,
    pub summary: String,
    pub player_visible_signal: String,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default)]
    pub relationship_refs: Vec<String>,
    #[serde(default)]
    pub consequence_refs: Vec<String>,
    #[serde(default)]
    pub commitment_refs: Vec<String>,
    #[serde(default)]
    pub unresolved_ask_refs: Vec<String>,
    #[serde(default)]
    pub commitment: Option<SocialCommitment>,
    #[serde(default)]
    pub unresolved_ask: Option<UnresolvedSocialAsk>,
    #[serde(default)]
    pub leverage: Option<ConversationLeverage>,
    #[serde(default)]
    pub closure: Option<SocialExchangeClosure>,
    pub recorded_at: String,
}

pub fn prepare_social_exchange_event_plan(
    current: &SocialExchangePacket,
    response: &AgentTurnResponse,
    consequences: &ConsequenceSpinePacket,
) -> Result<SocialExchangeEventPlan> {
    let mut records = Vec::new();
    let recorded_at = Utc::now().to_rfc3339();
    let mut event_context = current.clone();
    if let Some(proposal) = &response.social_exchange_proposal {
        validate_social_exchange_proposal(current, proposal, Some(response.turn_id.as_str()))?;
        event_context.turn_id.clone_from(&proposal.turn_id);
        for exchange in &proposal.exchanges {
            records.push(record_from_exchange(
                &event_context,
                exchange,
                records.len(),
                recorded_at.as_str(),
            ));
        }
        for commitment in &proposal.commitments {
            records.push(record_from_commitment(
                &event_context,
                commitment,
                records.len(),
                recorded_at.as_str(),
            ));
        }
        for ask in &proposal.unresolved_asks {
            records.push(record_from_ask(
                &event_context,
                ask,
                records.len(),
                recorded_at.as_str(),
            ));
        }
        for leverage in &proposal.leverage_updates {
            records.push(record_from_leverage(
                &event_context,
                leverage,
                records.len(),
                recorded_at.as_str(),
            ));
        }
        for closure in &proposal.paid_off_or_closed {
            records.push(record_from_closure(
                &event_context,
                closure,
                records.len(),
                recorded_at.as_str(),
            ));
        }
    }
    let existing_refs = records
        .iter()
        .flat_map(|record| record.source_refs.iter().map(String::as_str))
        .collect::<BTreeSet<_>>();
    let mut derived = derive_social_exchange_records(
        &event_context,
        response,
        consequences,
        records.len(),
        &existing_refs,
        recorded_at.as_str(),
    );
    records.append(&mut derived);
    Ok(SocialExchangeEventPlan {
        world_id: event_context.world_id.clone(),
        turn_id: event_context.turn_id.clone(),
        records,
    })
}

pub fn audit_social_exchange_contract(
    context: &PromptContextPacket,
    current: &SocialExchangePacket,
    response: &AgentTurnResponse,
) -> Result<()> {
    let Some(proposal) = &response.social_exchange_proposal else {
        return Ok(());
    };
    validate_social_exchange_proposal(current, proposal, Some(response.turn_id.as_str()))?;
    let mut visible_refs = collect_strings(&serde_json::to_value(&context.visible_context)?);
    extend_response_visible_refs(&mut visible_refs, response);
    let known_refs = collect_known_social_refs(context, response);
    for exchange in &proposal.exchanges {
        require_known_ref(
            &known_refs,
            exchange.actor_ref.as_str(),
            "exchange actor_ref",
        )?;
        require_known_ref(
            &known_refs,
            exchange.target_ref.as_str(),
            "exchange target_ref",
        )?;
        require_visible_refs(&visible_refs, "social exchange", &exchange.source_refs)?;
        reject_hidden_text(context, exchange.summary.as_str())?;
        reject_hidden_text(context, exchange.player_visible_signal.as_str())?;
        if matches!(
            exchange.intensity_after,
            SocialIntensity::High | SocialIntensity::Crisis
        ) && exchange.relationship_refs.is_empty()
            && exchange.consequence_refs.is_empty()
            && exchange.commitment_refs.is_empty()
            && exchange.unresolved_ask_refs.is_empty()
        {
            bail!("high/crisis social exchange requires an integration hook");
        }
    }
    for commitment in &proposal.commitments {
        require_known_ref(
            &known_refs,
            commitment.actor_ref.as_str(),
            "commitment actor_ref",
        )?;
        require_known_ref(
            &known_refs,
            commitment.target_ref.as_str(),
            "commitment target_ref",
        )?;
        require_visible_refs(&visible_refs, "social commitment", &commitment.source_refs)?;
        if commitment.condition.trim().is_empty() {
            bail!("social commitment requires condition");
        }
        reject_hidden_text(context, commitment.summary.as_str())?;
        reject_hidden_text(context, commitment.condition.as_str())?;
    }
    for ask in &proposal.unresolved_asks {
        require_known_ref(
            &known_refs,
            ask.asked_by_ref.as_str(),
            "unresolved ask asked_by_ref",
        )?;
        require_known_ref(
            &known_refs,
            ask.asked_to_ref.as_str(),
            "unresolved ask asked_to_ref",
        )?;
        require_visible_refs(&visible_refs, "unresolved social ask", &ask.source_refs)?;
        reject_hidden_text(context, ask.question_summary.as_str())?;
        reject_hidden_text(context, ask.last_response.as_str())?;
        if current
            .unresolved_asks
            .iter()
            .any(|current_ask| current_ask.ask_id == ask.ask_id)
            && ask.source_refs.len() < 2
        {
            bail!("reopening unresolved social ask requires new evidence or leverage");
        }
    }
    for leverage in &proposal.leverage_updates {
        require_known_ref(
            &known_refs,
            leverage.holder_ref.as_str(),
            "leverage holder_ref",
        )?;
        require_known_ref(
            &known_refs,
            leverage.target_ref.as_str(),
            "leverage target_ref",
        )?;
        require_visible_refs(
            &visible_refs,
            "conversation leverage",
            &leverage.source_refs,
        )?;
        reject_hidden_text(context, leverage.summary.as_str())?;
    }
    for closure in &proposal.paid_off_or_closed {
        require_visible_refs(&visible_refs, "social closure", &closure.evidence_refs)?;
        reject_hidden_text(context, closure.summary.as_str())?;
    }
    for note in &proposal.ephemeral_social_notes {
        require_visible_refs(&visible_refs, "ephemeral social note", &note.evidence_refs)?;
        reject_hidden_text(context, note.note.as_str())?;
    }
    Ok(())
}

fn validate_social_exchange_proposal(
    current: &SocialExchangePacket,
    proposal: &SocialExchangeProposal,
    expected_turn_id: Option<&str>,
) -> Result<()> {
    if proposal.schema_version != SOCIAL_EXCHANGE_PROPOSAL_SCHEMA_VERSION {
        bail!("social exchange proposal schema_version mismatch");
    }
    let next_turn_id = social_exchange_next_turn_id(current.turn_id.as_str()).ok();
    let turn_matches = proposal.turn_id == current.turn_id
        || next_turn_id
            .as_deref()
            .is_some_and(|turn_id| proposal.turn_id == turn_id)
        || expected_turn_id.is_some_and(|turn_id| proposal.turn_id == turn_id);
    if proposal.world_id != current.world_id || !turn_matches {
        bail!("social exchange proposal world_id/turn_id mismatch");
    }
    for exchange in &proposal.exchanges {
        if exchange.source_refs.is_empty()
            || exchange.summary.trim().is_empty()
            || exchange.player_visible_signal.trim().is_empty()
        {
            bail!("social exchange requires source_refs, summary, and player_visible_signal");
        }
    }
    for commitment in &proposal.commitments {
        if commitment.source_refs.is_empty()
            || commitment.summary.trim().is_empty()
            || commitment.condition.trim().is_empty()
        {
            bail!("social commitment requires source_refs, summary, and condition");
        }
    }
    Ok(())
}

fn social_exchange_next_turn_id(turn_id: &str) -> Result<String> {
    let suffix = turn_id
        .strip_prefix("turn_")
        .context("social exchange current turn_id missing turn_ prefix")?;
    let index: u64 = suffix
        .parse()
        .context("social exchange current turn_id suffix is not numeric")?;
    Ok(format!("turn_{:04}", index + 1))
}

fn record_from_exchange(
    current: &SocialExchangePacket,
    exchange: &SocialExchangeMutation,
    index: usize,
    recorded_at: &str,
) -> SocialExchangeEventRecord {
    SocialExchangeEventRecord {
        schema_version: SOCIAL_EXCHANGE_EVENT_SCHEMA_VERSION.to_owned(),
        world_id: current.world_id.clone(),
        turn_id: current.turn_id.clone(),
        event_id: format!("social_exchange_event:{}:{index:02}", current.turn_id),
        actor_ref: exchange.actor_ref.clone(),
        target_ref: exchange.target_ref.clone(),
        act_kind: exchange.act_kind,
        stance_after: exchange.stance_after,
        intensity_after: exchange.intensity_after,
        summary: exchange.summary.clone(),
        player_visible_signal: exchange.player_visible_signal.clone(),
        source_refs: exchange.source_refs.clone(),
        relationship_refs: exchange.relationship_refs.clone(),
        consequence_refs: exchange.consequence_refs.clone(),
        commitment_refs: exchange.commitment_refs.clone(),
        unresolved_ask_refs: exchange.unresolved_ask_refs.clone(),
        commitment: None,
        unresolved_ask: None,
        leverage: None,
        closure: None,
        recorded_at: recorded_at.to_owned(),
    }
}

fn record_from_commitment(
    current: &SocialExchangePacket,
    commitment: &SocialCommitmentMutation,
    index: usize,
    recorded_at: &str,
) -> SocialExchangeEventRecord {
    let social_commitment = SocialCommitment {
        schema_version: SOCIAL_COMMITMENT_SCHEMA_VERSION.to_owned(),
        commitment_id: commitment.commitment_id.clone(),
        actor_ref: commitment.actor_ref.clone(),
        target_ref: commitment.target_ref.clone(),
        kind: commitment.kind,
        status: SocialCommitmentStatus::Active,
        summary: commitment.summary.clone(),
        condition: commitment.condition.clone(),
        due_window: commitment.due_window,
        source_refs: commitment.source_refs.clone(),
        consequence_refs: commitment.consequence_refs.clone(),
    };
    SocialExchangeEventRecord {
        schema_version: SOCIAL_EXCHANGE_EVENT_SCHEMA_VERSION.to_owned(),
        world_id: current.world_id.clone(),
        turn_id: current.turn_id.clone(),
        event_id: format!("social_exchange_event:{}:{index:02}", current.turn_id),
        actor_ref: commitment.actor_ref.clone(),
        target_ref: commitment.target_ref.clone(),
        act_kind: act_for_commitment(commitment.kind),
        stance_after: stance_for_commitment(commitment.kind),
        intensity_after: SocialIntensity::Medium,
        summary: commitment.summary.clone(),
        player_visible_signal: commitment.condition.clone(),
        source_refs: commitment.source_refs.clone(),
        relationship_refs: Vec::new(),
        consequence_refs: commitment.consequence_refs.clone(),
        commitment_refs: vec![commitment.commitment_id.clone()],
        unresolved_ask_refs: Vec::new(),
        commitment: Some(social_commitment),
        unresolved_ask: None,
        leverage: None,
        closure: None,
        recorded_at: recorded_at.to_owned(),
    }
}

fn record_from_ask(
    current: &SocialExchangePacket,
    ask: &UnresolvedAskMutation,
    index: usize,
    recorded_at: &str,
) -> SocialExchangeEventRecord {
    let unresolved_ask = UnresolvedSocialAsk {
        schema_version: UNRESOLVED_SOCIAL_ASK_SCHEMA_VERSION.to_owned(),
        ask_id: ask.ask_id.clone(),
        asked_by_ref: ask.asked_by_ref.clone(),
        asked_to_ref: ask.asked_to_ref.clone(),
        question_summary: ask.question_summary.clone(),
        current_status: ask.current_status,
        last_response: ask.last_response.clone(),
        allowed_next_moves: ask.allowed_next_moves.clone(),
        blocked_repetitions: ask.blocked_repetitions.clone(),
        source_refs: ask.source_refs.clone(),
    };
    SocialExchangeEventRecord {
        schema_version: SOCIAL_EXCHANGE_EVENT_SCHEMA_VERSION.to_owned(),
        world_id: current.world_id.clone(),
        turn_id: current.turn_id.clone(),
        event_id: format!("social_exchange_event:{}:{index:02}", current.turn_id),
        actor_ref: ask.asked_to_ref.clone(),
        target_ref: ask.asked_by_ref.clone(),
        act_kind: act_for_ask_status(ask.current_status),
        stance_after: stance_for_ask_status(ask.current_status),
        intensity_after: SocialIntensity::Medium,
        summary: ask.question_summary.clone(),
        player_visible_signal: ask.last_response.clone(),
        source_refs: ask.source_refs.clone(),
        relationship_refs: Vec::new(),
        consequence_refs: Vec::new(),
        commitment_refs: Vec::new(),
        unresolved_ask_refs: vec![ask.ask_id.clone()],
        commitment: None,
        unresolved_ask: Some(unresolved_ask),
        leverage: None,
        closure: None,
        recorded_at: recorded_at.to_owned(),
    }
}

fn record_from_leverage(
    current: &SocialExchangePacket,
    leverage: &ConversationLeverageMutation,
    index: usize,
    recorded_at: &str,
) -> SocialExchangeEventRecord {
    let conversation_leverage = ConversationLeverage {
        schema_version: CONVERSATION_LEVERAGE_SCHEMA_VERSION.to_owned(),
        leverage_id: leverage.leverage_id.clone(),
        holder_ref: leverage.holder_ref.clone(),
        target_ref: leverage.target_ref.clone(),
        leverage_kind: leverage.leverage_kind,
        summary: leverage.summary.clone(),
        expires: leverage.expires,
        source_refs: leverage.source_refs.clone(),
    };
    SocialExchangeEventRecord {
        schema_version: SOCIAL_EXCHANGE_EVENT_SCHEMA_VERSION.to_owned(),
        world_id: current.world_id.clone(),
        turn_id: current.turn_id.clone(),
        event_id: format!("social_exchange_event:{}:{index:02}", current.turn_id),
        actor_ref: leverage.holder_ref.clone(),
        target_ref: leverage.target_ref.clone(),
        act_kind: SocialExchangeActKind::Offer,
        stance_after: DialogueStanceKind::Bargaining,
        intensity_after: SocialIntensity::Medium,
        summary: leverage.summary.clone(),
        player_visible_signal: leverage.summary.clone(),
        source_refs: leverage.source_refs.clone(),
        relationship_refs: Vec::new(),
        consequence_refs: Vec::new(),
        commitment_refs: Vec::new(),
        unresolved_ask_refs: Vec::new(),
        commitment: None,
        unresolved_ask: None,
        leverage: Some(conversation_leverage),
        closure: None,
        recorded_at: recorded_at.to_owned(),
    }
}

fn record_from_closure(
    current: &SocialExchangePacket,
    closure: &SocialExchangeClosure,
    index: usize,
    recorded_at: &str,
) -> SocialExchangeEventRecord {
    SocialExchangeEventRecord {
        schema_version: SOCIAL_EXCHANGE_EVENT_SCHEMA_VERSION.to_owned(),
        world_id: current.world_id.clone(),
        turn_id: current.turn_id.clone(),
        event_id: format!("social_exchange_event:{}:{index:02}", current.turn_id),
        actor_ref: "social:closure".to_owned(),
        target_ref: "player".to_owned(),
        act_kind: SocialExchangeActKind::Answer,
        stance_after: DialogueStanceKind::NeutralProcedure,
        intensity_after: SocialIntensity::Trace,
        summary: closure.summary.clone(),
        player_visible_signal: closure.summary.clone(),
        source_refs: closure.evidence_refs.clone(),
        relationship_refs: Vec::new(),
        consequence_refs: Vec::new(),
        commitment_refs: Vec::new(),
        unresolved_ask_refs: vec![closure.ref_id.clone()],
        commitment: None,
        unresolved_ask: None,
        leverage: None,
        closure: Some(closure.clone()),
        recorded_at: recorded_at.to_owned(),
    }
}

fn derive_social_exchange_records(
    current: &SocialExchangePacket,
    response: &AgentTurnResponse,
    consequences: &ConsequenceSpinePacket,
    offset: usize,
    existing_refs: &BTreeSet<&str>,
    recorded_at: &str,
) -> Vec<SocialExchangeEventRecord> {
    let mut records = Vec::new();
    for (index, update) in response.relationship_updates.iter().enumerate() {
        let source_refs = update
            .evidence_refs
            .iter()
            .filter(|reference| !existing_refs.contains(reference.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if source_refs.is_empty() {
            continue;
        }
        records.push(derived_relationship_record(
            current,
            update,
            offset + records.len(),
            index,
            source_refs,
            recorded_at,
        ));
    }
    if let Some(resolution) = &response.resolution_proposal {
        records.extend(derive_from_resolution(
            current,
            resolution,
            offset + records.len(),
            existing_refs,
            recorded_at,
        ));
    }
    for consequence in &consequences.active {
        if !is_social_consequence(consequence) {
            continue;
        }
        let source_refs = consequence
            .source_refs
            .iter()
            .filter(|reference| !existing_refs.contains(reference.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if source_refs.is_empty() {
            continue;
        }
        records.push(derived_consequence_record(
            current,
            consequence,
            offset + records.len(),
            source_refs,
            recorded_at,
        ));
    }
    records
}

fn derived_relationship_record(
    current: &SocialExchangePacket,
    update: &AgentRelationshipUpdate,
    index: usize,
    relation_index: usize,
    source_refs: Vec<String>,
    recorded_at: &str,
) -> SocialExchangeEventRecord {
    let relation = update.relation_kind.to_ascii_lowercase();
    let (stance_after, act_kind, signal) = if relation.contains("suspicion") {
        (
            DialogueStanceKind::WaryTesting,
            SocialExchangeActKind::Test,
            "상대의 경계가 다음 대화의 출발점으로 남는다.",
        )
    } else if relation.contains("debt") {
        (
            DialogueStanceKind::Indebted,
            SocialExchangeActKind::Promise,
            "대화 안에 빚이나 갚아야 할 반응이 남는다.",
        )
    } else if relation.contains("fear") {
        (
            DialogueStanceKind::Threatening,
            SocialExchangeActKind::Threaten,
            "두려움이 대화 거리를 바꾼다.",
        )
    } else if relation.contains("trust") {
        (
            DialogueStanceKind::GuardedHelpful,
            SocialExchangeActKind::Accept,
            "조심스러운 협조 여지가 생긴다.",
        )
    } else {
        (
            DialogueStanceKind::NeutralProcedure,
            SocialExchangeActKind::Answer,
            "관계 변화가 대화의 톤을 조금 바꾼다.",
        )
    };
    SocialExchangeEventRecord {
        schema_version: SOCIAL_EXCHANGE_EVENT_SCHEMA_VERSION.to_owned(),
        world_id: current.world_id.clone(),
        turn_id: current.turn_id.clone(),
        event_id: format!("social_exchange_event:{}:{index:02}", current.turn_id),
        actor_ref: update.source_entity_id.clone(),
        target_ref: update.target_entity_id.clone(),
        act_kind,
        stance_after,
        intensity_after: SocialIntensity::Medium,
        summary: update.summary.clone(),
        player_visible_signal: signal.to_owned(),
        source_refs,
        relationship_refs: vec![format!("relationship_update:{relation_index}")],
        consequence_refs: Vec::new(),
        commitment_refs: Vec::new(),
        unresolved_ask_refs: Vec::new(),
        commitment: None,
        unresolved_ask: None,
        leverage: None,
        closure: None,
        recorded_at: recorded_at.to_owned(),
    }
}

#[allow(clippy::too_many_lines)]
fn derive_from_resolution(
    current: &SocialExchangePacket,
    resolution: &ResolutionProposal,
    offset: usize,
    existing_refs: &BTreeSet<&str>,
    recorded_at: &str,
) -> Vec<SocialExchangeEventRecord> {
    let mut records = Vec::new();
    for (index, gate) in resolution.gate_results.iter().enumerate() {
        if gate.gate_kind != GateKind::SocialPermission
            || gate.visibility != ResolutionVisibility::PlayerVisible
            || gate
                .evidence_refs
                .iter()
                .any(|reference| existing_refs.contains(reference.as_str()))
        {
            continue;
        }
        let (act_kind, stance_after, signal) = match gate.status {
            GateStatus::Blocked => (
                SocialExchangeActKind::Refuse,
                DialogueStanceKind::Withholding,
                "사회적 허가가 막혀 같은 요구를 반복할 수 없다.",
            ),
            GateStatus::CostImposed => (
                SocialExchangeActKind::CounterOffer,
                DialogueStanceKind::Bargaining,
                "대화가 조건이나 비용을 요구하는 상태로 남는다.",
            ),
            GateStatus::UnknownNeedsProbe => (
                SocialExchangeActKind::Test,
                DialogueStanceKind::WaryTesting,
                "상대가 먼저 확인이나 시험을 요구한다.",
            ),
            GateStatus::Softened => (
                SocialExchangeActKind::GrantPermission,
                DialogueStanceKind::GuardedHelpful,
                "허가는 열렸지만 조심스러운 조건이 남는다.",
            ),
            GateStatus::Passed => (
                SocialExchangeActKind::Accept,
                DialogueStanceKind::Cooperative,
                "대화의 허가가 열려 협조가 가능해졌다.",
            ),
        };
        records.push(SocialExchangeEventRecord {
            schema_version: SOCIAL_EXCHANGE_EVENT_SCHEMA_VERSION.to_owned(),
            world_id: current.world_id.clone(),
            turn_id: current.turn_id.clone(),
            event_id: format!(
                "social_exchange_event:{}:{:02}",
                current.turn_id,
                offset + records.len()
            ),
            actor_ref: gate.gate_ref.clone(),
            target_ref: "player".to_owned(),
            act_kind,
            stance_after,
            intensity_after: SocialIntensity::Medium,
            summary: gate.reason.clone(),
            player_visible_signal: signal.to_owned(),
            source_refs: gate.evidence_refs.clone(),
            relationship_refs: vec![gate.gate_ref.clone()],
            consequence_refs: Vec::new(),
            commitment_refs: Vec::new(),
            unresolved_ask_refs: if gate.status == GateStatus::Blocked {
                vec![format!(
                    "ask:{}:social_permission:{index}",
                    resolution.turn_id
                )]
            } else {
                Vec::new()
            },
            commitment: None,
            unresolved_ask: None,
            leverage: None,
            closure: None,
            recorded_at: recorded_at.to_owned(),
        });
    }
    if matches!(
        resolution.outcome.kind,
        ResolutionOutcomeKind::Blocked | ResolutionOutcomeKind::CostlySuccess
    ) && !resolution.outcome.evidence_refs.is_empty()
    {
        let (act_kind, stance_after, signal) =
            if resolution.outcome.kind == ResolutionOutcomeKind::Blocked {
                (
                    SocialExchangeActKind::Refuse,
                    DialogueStanceKind::Withholding,
                    "막힌 대화가 다음 시도에도 제약으로 남는다.",
                )
            } else {
                (
                    SocialExchangeActKind::CounterOffer,
                    DialogueStanceKind::Bargaining,
                    "성공했지만 대화 안에 갚을 비용이 남았다.",
                )
            };
        records.push(SocialExchangeEventRecord {
            schema_version: SOCIAL_EXCHANGE_EVENT_SCHEMA_VERSION.to_owned(),
            world_id: current.world_id.clone(),
            turn_id: current.turn_id.clone(),
            event_id: format!(
                "social_exchange_event:{}:{:02}",
                current.turn_id,
                offset + records.len()
            ),
            actor_ref: "scene:social_outcome".to_owned(),
            target_ref: "player".to_owned(),
            act_kind,
            stance_after,
            intensity_after: SocialIntensity::Medium,
            summary: resolution.outcome.summary.clone(),
            player_visible_signal: signal.to_owned(),
            source_refs: resolution.outcome.evidence_refs.clone(),
            relationship_refs: Vec::new(),
            consequence_refs: Vec::new(),
            commitment_refs: Vec::new(),
            unresolved_ask_refs: Vec::new(),
            commitment: None,
            unresolved_ask: None,
            leverage: None,
            closure: None,
            recorded_at: recorded_at.to_owned(),
        });
    }
    records
}

fn derived_consequence_record(
    current: &SocialExchangePacket,
    consequence: &ActiveConsequence,
    index: usize,
    source_refs: Vec<String>,
    recorded_at: &str,
) -> SocialExchangeEventRecord {
    SocialExchangeEventRecord {
        schema_version: SOCIAL_EXCHANGE_EVENT_SCHEMA_VERSION.to_owned(),
        world_id: current.world_id.clone(),
        turn_id: current.turn_id.clone(),
        event_id: format!("social_exchange_event:{}:{index:02}", current.turn_id),
        actor_ref: consequence
            .linked_entity_refs
            .first()
            .cloned()
            .unwrap_or_else(|| "scene:social_consequence".to_owned()),
        target_ref: "player".to_owned(),
        act_kind: SocialExchangeActKind::Test,
        stance_after: match consequence.kind {
            ConsequenceKind::SocialDebt => DialogueStanceKind::Indebted,
            ConsequenceKind::SuspicionRaised => DialogueStanceKind::WaryTesting,
            ConsequenceKind::TrustShift => DialogueStanceKind::GuardedHelpful,
            _ => DialogueStanceKind::NeutralProcedure,
        },
        intensity_after: intensity_for_consequence(consequence),
        summary: consequence.summary.clone(),
        player_visible_signal: consequence.player_visible_signal.clone(),
        source_refs,
        relationship_refs: Vec::new(),
        consequence_refs: vec![consequence.consequence_id.clone()],
        commitment_refs: Vec::new(),
        unresolved_ask_refs: Vec::new(),
        commitment: None,
        unresolved_ask: None,
        leverage: None,
        closure: None,
        recorded_at: recorded_at.to_owned(),
    }
}

pub fn append_social_exchange_event_plan(
    world_dir: &Path,
    plan: &SocialExchangeEventPlan,
) -> Result<()> {
    for record in &plan.records {
        append_jsonl(&world_dir.join(SOCIAL_EXCHANGE_EVENTS_FILENAME), record)?;
    }
    Ok(())
}

pub fn rebuild_social_exchange(
    world_dir: &Path,
    base_packet: &SocialExchangePacket,
) -> Result<SocialExchangePacket> {
    let records = load_social_exchange_event_records(world_dir)?;
    let mut stances_by_id = BTreeMap::new();
    let mut commitments_by_id = BTreeMap::new();
    let mut asks_by_id = BTreeMap::new();
    let mut leverage_by_id = BTreeMap::new();
    let mut recent_exchanges = Vec::new();
    let mut closed_refs = BTreeSet::new();
    for record in records {
        if let Some(closure) = &record.closure {
            closed_refs.insert(closure.ref_id.clone());
        }
        let stance_id = format!("stance:{}->{}", record.actor_ref, record.target_ref);
        stances_by_id.insert(
            stance_id.clone(),
            DialogueStance {
                schema_version: SOCIAL_EXCHANGE_STANCE_SCHEMA_VERSION.to_owned(),
                stance_id,
                actor_ref: record.actor_ref.clone(),
                target_ref: record.target_ref.clone(),
                stance: record.stance_after,
                intensity: record.intensity_after,
                summary: record.summary.clone(),
                player_visible_signal: record.player_visible_signal.clone(),
                source_refs: record.source_refs.clone(),
                relationship_refs: record.relationship_refs.clone(),
                consequence_refs: record.consequence_refs.clone(),
                opened_turn_id: record.turn_id.clone(),
                last_changed_turn_id: record.turn_id.clone(),
            },
        );
        if let Some(commitment) = record.commitment.clone() {
            commitments_by_id.insert(commitment.commitment_id.clone(), commitment);
        }
        if let Some(ask) = record.unresolved_ask.clone() {
            asks_by_id.insert(ask.ask_id.clone(), ask);
        }
        if let Some(leverage) = record.leverage.clone() {
            leverage_by_id.insert(leverage.leverage_id.clone(), leverage);
        }
        recent_exchanges.push(SocialExchangeMemory {
            event_id: record.event_id,
            actor_ref: record.actor_ref,
            target_ref: record.target_ref,
            act_kind: record.act_kind,
            summary: record.summary,
            player_visible_signal: record.player_visible_signal,
            source_refs: record.source_refs,
        });
    }
    let mut active_stances = stances_by_id.into_values().rev().collect::<Vec<_>>();
    active_stances.truncate(STANCE_BUDGET);
    let mut active_commitments = commitments_by_id
        .into_values()
        .filter(|commitment| {
            commitment.status == SocialCommitmentStatus::Active
                && !closed_refs.contains(&commitment.commitment_id)
        })
        .rev()
        .collect::<Vec<_>>();
    active_commitments.truncate(COMMITMENT_BUDGET);
    let mut unresolved_asks = asks_by_id
        .into_values()
        .filter(|ask| {
            !matches!(
                ask.current_status,
                AskStatus::Answered | AskStatus::Obsolete
            ) && !closed_refs.contains(&ask.ask_id)
        })
        .rev()
        .collect::<Vec<_>>();
    unresolved_asks.truncate(ASK_BUDGET);
    let mut leverage = leverage_by_id.into_values().rev().collect::<Vec<_>>();
    leverage.truncate(LEVERAGE_BUDGET);
    recent_exchanges.reverse();
    recent_exchanges.truncate(EXCHANGE_MEMORY_BUDGET);
    let packet = SocialExchangePacket {
        schema_version: SOCIAL_EXCHANGE_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: base_packet.world_id.clone(),
        turn_id: base_packet.turn_id.clone(),
        active_stances,
        active_commitments,
        unresolved_asks,
        recent_exchanges,
        leverage,
        compiler_policy: SocialExchangePolicy::default(),
    };
    write_json(&world_dir.join(DIALOGUE_STANCE_FILENAME), &packet)?;
    Ok(packet)
}

pub fn load_social_exchange_state(
    world_dir: &Path,
    fallback: SocialExchangePacket,
) -> Result<SocialExchangePacket> {
    let path = world_dir.join(DIALOGUE_STANCE_FILENAME);
    if path.is_file() {
        return read_json(&path);
    }
    Ok(fallback)
}

fn load_social_exchange_event_records(world_dir: &Path) -> Result<Vec<SocialExchangeEventRecord>> {
    let path = world_dir.join(SOCIAL_EXCHANGE_EVENTS_FILENAME);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str::<SocialExchangeEventRecord>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn act_for_commitment(kind: SocialCommitmentKind) -> SocialExchangeActKind {
    match kind {
        SocialCommitmentKind::Promise | SocialCommitmentKind::Debt => {
            SocialExchangeActKind::Promise
        }
        SocialCommitmentKind::Demand | SocialCommitmentKind::Price => SocialExchangeActKind::Demand,
        SocialCommitmentKind::ConditionalPermission => SocialExchangeActKind::GrantPermission,
        SocialCommitmentKind::Truce => SocialExchangeActKind::Accept,
        SocialCommitmentKind::Threat => SocialExchangeActKind::Threaten,
    }
}

fn stance_for_commitment(kind: SocialCommitmentKind) -> DialogueStanceKind {
    match kind {
        SocialCommitmentKind::Promise | SocialCommitmentKind::Debt => DialogueStanceKind::Indebted,
        SocialCommitmentKind::Demand | SocialCommitmentKind::Price => {
            DialogueStanceKind::Bargaining
        }
        SocialCommitmentKind::ConditionalPermission => DialogueStanceKind::GuardedHelpful,
        SocialCommitmentKind::Truce => DialogueStanceKind::Appeasing,
        SocialCommitmentKind::Threat => DialogueStanceKind::Threatening,
    }
}

fn act_for_ask_status(status: AskStatus) -> SocialExchangeActKind {
    match status {
        AskStatus::Open => SocialExchangeActKind::Ask,
        AskStatus::Answered | AskStatus::Obsolete => SocialExchangeActKind::Answer,
        AskStatus::Evaded => SocialExchangeActKind::Evade,
        AskStatus::Refused => SocialExchangeActKind::Refuse,
        AskStatus::Conditional => SocialExchangeActKind::RevealConditionally,
    }
}

fn stance_for_ask_status(status: AskStatus) -> DialogueStanceKind {
    match status {
        AskStatus::Open => DialogueStanceKind::WaryTesting,
        AskStatus::Answered => DialogueStanceKind::Cooperative,
        AskStatus::Evaded => DialogueStanceKind::Evasive,
        AskStatus::Refused => DialogueStanceKind::Withholding,
        AskStatus::Conditional => DialogueStanceKind::Bargaining,
        AskStatus::Obsolete => DialogueStanceKind::NeutralProcedure,
    }
}

const fn is_social_consequence(consequence: &ActiveConsequence) -> bool {
    matches!(
        consequence.kind,
        ConsequenceKind::SocialDebt
            | ConsequenceKind::TrustShift
            | ConsequenceKind::SuspicionRaised
    )
}

fn intensity_for_consequence(consequence: &ActiveConsequence) -> SocialIntensity {
    match consequence.severity {
        crate::consequence_spine::ConsequenceSeverity::Trace => SocialIntensity::Trace,
        crate::consequence_spine::ConsequenceSeverity::Minor => SocialIntensity::Low,
        crate::consequence_spine::ConsequenceSeverity::Moderate => SocialIntensity::Medium,
        crate::consequence_spine::ConsequenceSeverity::Major => SocialIntensity::High,
        crate::consequence_spine::ConsequenceSeverity::Critical => SocialIntensity::Crisis,
    }
}

fn collect_known_social_refs(
    context: &PromptContextPacket,
    response: &AgentTurnResponse,
) -> BTreeSet<String> {
    let mut refs =
        collect_strings(&serde_json::to_value(&context.visible_context).unwrap_or_default());
    refs.insert("player".to_owned());
    refs.insert("scene:social_outcome".to_owned());
    refs.insert("scene:social_consequence".to_owned());
    refs.insert("social:closure".to_owned());
    for entity in &response.entity_updates {
        refs.insert(entity.entity_id.clone());
    }
    refs
}

fn extend_response_visible_refs(visible_refs: &mut BTreeSet<String>, response: &AgentTurnResponse) {
    for (index, block) in response.visible_scene.text_blocks.iter().enumerate() {
        visible_refs.insert(format!("visible_scene.text_blocks[{index}]"));
        if !block.trim().is_empty() {
            visible_refs.insert(block.clone());
        }
    }
    for choice in &response.next_choices {
        visible_refs.insert(format!("next_choices[slot={}]", choice.slot));
        visible_refs.insert(choice.tag.clone());
        visible_refs.insert(choice.intent.clone());
    }
    if let Some(canon_event) = &response.canon_event {
        visible_refs.insert(canon_event.summary.clone());
    }
}

fn require_known_ref(known_refs: &BTreeSet<String>, reference: &str, label: &str) -> Result<()> {
    if known_refs.contains(reference)
        || reference.starts_with("char:")
        || reference.starts_with("rel:")
        || reference.starts_with("faction:")
        || reference.starts_with("place:")
        || reference.starts_with("scene:")
        || reference == "player"
    {
        return Ok(());
    }
    bail!("{label} is not present in visible social context: {reference}");
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
    if collect_strings(&hidden)
        .into_iter()
        .filter(|hidden_text| hidden_text.chars().count() >= 8)
        .any(|hidden_text| text.contains(hidden_text.as_str()))
    {
        bail!("social exchange visible field contains hidden/adjudication-only text");
    }
    Ok(())
}

fn collect_strings(value: &serde_json::Value) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    collect_strings_into(value, &mut refs);
    insert_social_ref_aliases(&mut refs);
    refs
}

fn insert_social_ref_aliases(refs: &mut BTreeSet<String>) {
    let aliases = refs
        .iter()
        .filter_map(|item| relationship_ref_alias(item))
        .collect::<Vec<_>>();
    refs.extend(aliases);
}

fn relationship_ref_alias(item: &str) -> Option<String> {
    let body = item.strip_prefix("relationship:rel_")?;
    let (source, target) = body.split_once("-_")?;
    Some(format!(
        "rel:{}->{}",
        underscored_entity_ref(source)?,
        underscored_entity_ref(target)?
    ))
}

fn underscored_entity_ref(value: &str) -> Option<String> {
    let (prefix, suffix) = value.split_once('_')?;
    Some(format!("{prefix}:{suffix}"))
}

fn collect_strings_into(value: &serde_json::Value, refs: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::String(text) if !text.trim().is_empty() => {
            refs.insert(text.to_owned());
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_strings_into(item, refs);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                collect_strings_into(item, refs);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{NARRATIVE_SCENE_SCHEMA_VERSION, NarrativeScene};
    use crate::resolution::{
        ActionAmbiguity, ActionInputKind, ActionIntent, ChoicePlan, NarrativeBrief,
        RESOLUTION_PROPOSAL_SCHEMA_VERSION, ResolutionOutcome,
    };

    #[test]
    fn derives_social_exchange_from_social_permission_gate() -> anyhow::Result<()> {
        let packet = SocialExchangePacket {
            world_id: "world".to_owned(),
            turn_id: "turn_0003".to_owned(),
            ..SocialExchangePacket::default()
        };
        let mut response = sample_response();
        response.resolution_proposal = Some(sample_resolution());

        let plan = prepare_social_exchange_event_plan(
            &packet,
            &response,
            &ConsequenceSpinePacket::default(),
        )?;

        assert!(
            plan.records
                .iter()
                .any(|record| record.stance_after == DialogueStanceKind::Withholding)
        );
        Ok(())
    }

    #[test]
    fn materializes_stance_commitment_and_unresolved_ask() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let packet = SocialExchangePacket {
            world_id: "world".to_owned(),
            turn_id: "turn_0003".to_owned(),
            ..SocialExchangePacket::default()
        };
        let proposal = SocialExchangeProposal {
            schema_version: SOCIAL_EXCHANGE_PROPOSAL_SCHEMA_VERSION.to_owned(),
            world_id: "world".to_owned(),
            turn_id: "turn_0003".to_owned(),
            exchanges: vec![SocialExchangeMutation {
                actor_ref: "char:guard".to_owned(),
                target_ref: "player".to_owned(),
                act_kind: SocialExchangeActKind::Evade,
                stance_after: DialogueStanceKind::WaryTesting,
                intensity_after: SocialIntensity::Medium,
                summary: "문지기는 이유를 답하지 않고 신원을 요구한다.".to_owned(),
                player_visible_signal: "신원 확인이 먼저 요구된다.".to_owned(),
                source_refs: vec!["visible_scene.text_blocks[0]".to_owned()],
                relationship_refs: Vec::new(),
                consequence_refs: Vec::new(),
                commitment_refs: Vec::new(),
                unresolved_ask_refs: vec!["ask:gate".to_owned()],
            }],
            commitments: vec![SocialCommitmentMutation {
                commitment_id: "commitment:gate".to_owned(),
                actor_ref: "char:guard".to_owned(),
                target_ref: "player".to_owned(),
                kind: SocialCommitmentKind::ConditionalPermission,
                summary: "문지기는 신원을 밝히면 검문을 이어가겠다고 했다.".to_owned(),
                condition: "이름과 온 길을 말해야 한다.".to_owned(),
                due_window: SocialDueWindow::CurrentScene,
                source_refs: vec!["visible_scene.text_blocks[0]".to_owned()],
                consequence_refs: Vec::new(),
            }],
            unresolved_asks: vec![UnresolvedAskMutation {
                ask_id: "ask:gate".to_owned(),
                asked_by_ref: "player".to_owned(),
                asked_to_ref: "char:guard".to_owned(),
                question_summary: "문이 왜 일찍 닫혔는가?".to_owned(),
                current_status: AskStatus::Evaded,
                last_response: "규정이라는 말로 피했다.".to_owned(),
                allowed_next_moves: vec!["신원을 밝힌다".to_owned()],
                blocked_repetitions: vec!["같은 질문을 근거 없이 반복".to_owned()],
                source_refs: vec!["visible_scene.text_blocks[0]".to_owned()],
            }],
            leverage_updates: Vec::new(),
            paid_off_or_closed: Vec::new(),
            ephemeral_social_notes: Vec::new(),
        };
        let mut response = sample_response();
        response.social_exchange_proposal = Some(proposal);
        let plan = prepare_social_exchange_event_plan(
            &packet,
            &response,
            &ConsequenceSpinePacket::default(),
        )?;
        append_social_exchange_event_plan(temp.path(), &plan)?;
        let rebuilt = rebuild_social_exchange(temp.path(), &packet)?;

        assert_eq!(rebuilt.active_stances.len(), 1);
        assert_eq!(rebuilt.active_commitments.len(), 1);
        assert_eq!(rebuilt.unresolved_asks.len(), 1);
        assert!(temp.path().join(DIALOGUE_STANCE_FILENAME).is_file());
        Ok(())
    }

    fn sample_response() -> AgentTurnResponse {
        AgentTurnResponse {
            schema_version: crate::agent_bridge::AGENT_TURN_RESPONSE_SCHEMA_VERSION.to_owned(),
            world_id: "world".to_owned(),
            turn_id: "turn_0003".to_owned(),
            resolution_proposal: None,
            scene_director_proposal: None,
            consequence_proposal: None,
            social_exchange_proposal: None,
            encounter_proposal: None,
            visible_scene: NarrativeScene {
                schema_version: NARRATIVE_SCENE_SCHEMA_VERSION.to_owned(),
                speaker: None,
                text_blocks: vec!["문지기는 대답 대신 이름을 물었다.".to_owned()],
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
            hook_events: Vec::new(),
        }
    }

    fn sample_resolution() -> ResolutionProposal {
        ResolutionProposal {
            schema_version: RESOLUTION_PROPOSAL_SCHEMA_VERSION.to_owned(),
            world_id: "world".to_owned(),
            turn_id: "turn_0003".to_owned(),
            interpreted_intent: ActionIntent {
                input_kind: ActionInputKind::PresentedChoice,
                summary: "문지기에게 이유를 묻는다".to_owned(),
                target_refs: vec!["choice:1".to_owned()],
                pressure_refs: Vec::new(),
                evidence_refs: vec!["choice:1".to_owned()],
                ambiguity: ActionAmbiguity::Clear,
            },
            outcome: ResolutionOutcome {
                kind: ResolutionOutcomeKind::Blocked,
                summary: "문지기는 이유를 말하지 않는다.".to_owned(),
                evidence_refs: vec!["choice:1".to_owned()],
            },
            gate_results: vec![crate::resolution::GateResult {
                gate_kind: GateKind::SocialPermission,
                gate_ref: "rel:guard".to_owned(),
                visibility: ResolutionVisibility::PlayerVisible,
                status: GateStatus::Blocked,
                reason: "문지기는 신원 확인 전에는 답하지 않는다.".to_owned(),
                evidence_refs: vec!["choice:1".to_owned()],
            }],
            proposed_effects: Vec::new(),
            process_ticks: Vec::new(),
            pressure_noop_reasons: Vec::new(),
            narrative_brief: NarrativeBrief {
                visible_summary: "답이 막힌다.".to_owned(),
                required_beats: Vec::new(),
                forbidden_visible_details: Vec::new(),
            },
            next_choice_plan: Vec::<ChoicePlan>::new(),
        }
    }
}
