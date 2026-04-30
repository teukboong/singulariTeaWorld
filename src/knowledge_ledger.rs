use crate::store::{append_jsonl, write_json};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

pub const KNOWLEDGE_CLAIM_SCHEMA_VERSION: &str = "singulari.knowledge_claim.v1";
pub const KNOWLEDGE_EVENT_SCHEMA_VERSION: &str = "singulari.knowledge_event.v1";
pub const KNOWLEDGE_LEDGER_STATE_SCHEMA_VERSION: &str = "singulari.knowledge_ledger_state.v1";
pub const KNOWLEDGE_EVENTS_FILENAME: &str = "knowledge_events.jsonl";
pub const KNOWLEDGE_LEDGER_FILENAME: &str = "knowledge_ledger.json";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeTier {
    WorldTrueHidden,
    #[default]
    PlayerObserved,
    PlayerInferred,
    Rumor,
    FalseBelief,
    Contradicted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TruthStatus {
    True,
    False,
    Unknown,
    Contested,
    Inferred,
    Rumored,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgeClaim {
    pub schema_version: String,
    pub claim_id: String,
    pub holder_ref: String,
    pub tier: KnowledgeTier,
    pub truth_status: TruthStatus,
    pub proposition: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgeLedgerState {
    pub schema_version: String,
    pub world_id: String,
    #[serde(default)]
    pub claims: Vec<KnowledgeClaim>,
    #[serde(default)]
    pub holders: Vec<KnowledgeHolderSnapshot>,
    #[serde(default)]
    pub conflicts: Vec<KnowledgeConflict>,
    pub rebuilt_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeHolderKind {
    Player,
    Npc,
    World,
    System,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgeHolderSnapshot {
    pub holder_ref: String,
    pub holder_kind: KnowledgeHolderKind,
    pub claim_count: usize,
    pub hidden_claim_count: usize,
    pub observed_claim_count: usize,
    pub inferred_claim_count: usize,
    pub rumor_claim_count: usize,
    pub false_belief_count: usize,
    pub contradicted_claim_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgeConflict {
    pub claim_id: String,
    pub holder_refs: Vec<String>,
    pub conflict_kinds: Vec<KnowledgeConflictKind>,
    pub propositions: Vec<String>,
    pub truth_statuses: Vec<TruthStatus>,
    pub tiers: Vec<KnowledgeTier>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeConflictKind {
    PropositionDisagreement,
    TruthStatusDisagreement,
    TierDisagreement,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgeEventRecord {
    pub schema_version: String,
    pub event_id: String,
    pub world_id: String,
    pub turn_id: String,
    pub claim_id: String,
    pub event_kind: KnowledgeEventKind,
    pub holder_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_holder_ref: Option<String>,
    pub tier: KnowledgeTier,
    pub truth_status: TruthStatus,
    pub proposition: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeEventKind {
    Observed,
    Inferred,
    Rumored,
    HiddenTruth,
    Confirmed,
    Contradicted,
    FalseBelief,
    Transferred,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgeClaimEventInput {
    pub world_id: String,
    pub turn_id: String,
    pub claim_id: String,
    pub holder_ref: String,
    pub proposition: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgeTransferInput {
    pub world_id: String,
    pub turn_id: String,
    pub claim_id: String,
    pub from_holder_ref: String,
    pub to_holder_ref: String,
    pub tier: KnowledgeTier,
    pub truth_status: TruthStatus,
    pub proposition: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerRenderPermission {
    DirectFact,
    UncertainInference,
    SourcedRumor,
    BeliefOnly,
    BlockedHiddenTruth,
}

#[must_use]
pub const fn player_render_permission(tier: KnowledgeTier) -> PlayerRenderPermission {
    match tier {
        KnowledgeTier::WorldTrueHidden => PlayerRenderPermission::BlockedHiddenTruth,
        KnowledgeTier::PlayerObserved => PlayerRenderPermission::DirectFact,
        KnowledgeTier::PlayerInferred => PlayerRenderPermission::UncertainInference,
        KnowledgeTier::Rumor => PlayerRenderPermission::SourcedRumor,
        KnowledgeTier::FalseBelief | KnowledgeTier::Contradicted => {
            PlayerRenderPermission::BeliefOnly
        }
    }
}

#[must_use]
pub const fn can_render_knowledge_tier_to_player(tier: KnowledgeTier) -> bool {
    !matches!(
        player_render_permission(tier),
        PlayerRenderPermission::BlockedHiddenTruth
    )
}

/// Persist a directly observed player/NPC knowledge claim.
///
/// # Errors
///
/// Returns an error when the claim input is incomplete or the knowledge event
/// cannot be appended.
pub fn observe_claim(
    world_dir: &Path,
    input: KnowledgeClaimEventInput,
) -> Result<KnowledgeEventRecord> {
    append_claim_event(
        world_dir,
        input,
        KnowledgeEventKind::Observed,
        KnowledgeTier::PlayerObserved,
        TruthStatus::True,
    )
}

/// Persist an inferred claim. Inferred claims are replayable but must render
/// with uncertainty on player-visible surfaces.
///
/// # Errors
///
/// Returns an error when the claim input is incomplete or the knowledge event
/// cannot be appended.
pub fn infer_claim(
    world_dir: &Path,
    input: KnowledgeClaimEventInput,
) -> Result<KnowledgeEventRecord> {
    append_claim_event(
        world_dir,
        input,
        KnowledgeEventKind::Inferred,
        KnowledgeTier::PlayerInferred,
        TruthStatus::Inferred,
    )
}

/// Persist a sourced rumor claim.
///
/// # Errors
///
/// Returns an error when the claim input is incomplete or the knowledge event
/// cannot be appended.
pub fn rumor_claim(
    world_dir: &Path,
    input: KnowledgeClaimEventInput,
) -> Result<KnowledgeEventRecord> {
    append_claim_event(
        world_dir,
        input,
        KnowledgeEventKind::Rumored,
        KnowledgeTier::Rumor,
        TruthStatus::Rumored,
    )
}

/// Persist a world-true hidden claim. Hidden truth may guide simulation but must
/// not render directly to player-visible surfaces.
///
/// # Errors
///
/// Returns an error when the claim input is incomplete or the knowledge event
/// cannot be appended.
pub fn hidden_truth_claim(
    world_dir: &Path,
    input: KnowledgeClaimEventInput,
) -> Result<KnowledgeEventRecord> {
    append_claim_event(
        world_dir,
        input,
        KnowledgeEventKind::HiddenTruth,
        KnowledgeTier::WorldTrueHidden,
        TruthStatus::True,
    )
}

/// Persist a false belief held by a specific holder.
///
/// # Errors
///
/// Returns an error when the claim input is incomplete or the knowledge event
/// cannot be appended.
pub fn false_belief_claim(
    world_dir: &Path,
    input: KnowledgeClaimEventInput,
) -> Result<KnowledgeEventRecord> {
    append_claim_event(
        world_dir,
        input,
        KnowledgeEventKind::FalseBelief,
        KnowledgeTier::FalseBelief,
        TruthStatus::False,
    )
}

/// Confirm a claim as observed truth for the holder.
///
/// # Errors
///
/// Returns an error when the claim input is incomplete or the knowledge event
/// cannot be appended.
pub fn confirm_claim(
    world_dir: &Path,
    input: KnowledgeClaimEventInput,
) -> Result<KnowledgeEventRecord> {
    append_claim_event(
        world_dir,
        input,
        KnowledgeEventKind::Confirmed,
        KnowledgeTier::PlayerObserved,
        TruthStatus::True,
    )
}

/// Mark a claim as contradicted for the holder.
///
/// # Errors
///
/// Returns an error when the claim input is incomplete or the knowledge event
/// cannot be appended.
pub fn contradict_claim(
    world_dir: &Path,
    input: KnowledgeClaimEventInput,
) -> Result<KnowledgeEventRecord> {
    append_claim_event(
        world_dir,
        input,
        KnowledgeEventKind::Contradicted,
        KnowledgeTier::Contradicted,
        TruthStatus::Contested,
    )
}

/// Transfer a claim between holders without collapsing their knowledge states.
///
/// # Errors
///
/// Returns an error when the transfer input is incomplete or the knowledge event
/// cannot be appended.
pub fn transfer_claim_between_holders(
    world_dir: &Path,
    input: KnowledgeTransferInput,
) -> Result<KnowledgeEventRecord> {
    validate_transfer_input(&input)?;
    append_knowledge_event(world_dir, |event_id, recorded_at| KnowledgeEventRecord {
        schema_version: KNOWLEDGE_EVENT_SCHEMA_VERSION.to_owned(),
        event_id,
        world_id: input.world_id,
        turn_id: input.turn_id,
        claim_id: input.claim_id,
        event_kind: KnowledgeEventKind::Transferred,
        holder_ref: input.to_holder_ref,
        from_holder_ref: Some(input.from_holder_ref),
        tier: input.tier,
        truth_status: input.truth_status,
        proposition: input.proposition,
        evidence_refs: input.evidence_refs,
        recorded_at,
    })
}

/// Load all persisted knowledge events for a world directory.
///
/// # Errors
///
/// Returns an error when the JSONL file cannot be read or parsed.
pub fn load_knowledge_events(world_dir: &Path) -> Result<Vec<KnowledgeEventRecord>> {
    let path = world_dir.join(KNOWLEDGE_EVENTS_FILENAME);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let raw =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    raw.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str::<KnowledgeEventRecord>(line)
                .with_context(|| format!("failed to parse {} line {}", path.display(), index + 1))
        })
        .collect()
}

/// Replay knowledge events into the latest per-holder claim state.
///
/// # Errors
///
/// Returns an error when event schema/world invariants are violated.
pub fn replay_knowledge_state(
    world_id: &str,
    events: &[KnowledgeEventRecord],
) -> Result<KnowledgeLedgerState> {
    let mut claims = BTreeMap::<(String, String), KnowledgeClaim>::new();
    for event in events {
        validate_event(world_id, event)?;
        let key = (event.holder_ref.clone(), event.claim_id.clone());
        claims.insert(
            key,
            KnowledgeClaim {
                schema_version: KNOWLEDGE_CLAIM_SCHEMA_VERSION.to_owned(),
                claim_id: event.claim_id.clone(),
                holder_ref: event.holder_ref.clone(),
                tier: event.tier,
                truth_status: event.truth_status,
                proposition: event.proposition.clone(),
                evidence_refs: event.evidence_refs.clone(),
            },
        );
    }
    let claims = claims.into_values().collect::<Vec<_>>();
    let holders = compile_holder_snapshots(&claims);
    let conflicts = compile_knowledge_conflicts(&claims);
    Ok(KnowledgeLedgerState {
        schema_version: KNOWLEDGE_LEDGER_STATE_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        claims,
        holders,
        conflicts,
        rebuilt_at: Utc::now().to_rfc3339(),
    })
}

/// Rebuild and persist `knowledge_ledger.json` from `knowledge_events.jsonl`.
///
/// # Errors
///
/// Returns an error when events cannot be loaded/replayed or state cannot be
/// written.
pub fn rebuild_knowledge_ledger(world_dir: &Path, world_id: &str) -> Result<KnowledgeLedgerState> {
    let events = load_knowledge_events(world_dir)?;
    let state = replay_knowledge_state(world_id, &events)?;
    write_json(&world_dir.join(KNOWLEDGE_LEDGER_FILENAME), &state)?;
    Ok(state)
}

#[must_use]
pub fn render_rule_for_player(tier: KnowledgeTier) -> &'static str {
    match player_render_permission(tier) {
        PlayerRenderPermission::DirectFact => "may render as observed fact",
        PlayerRenderPermission::UncertainInference => "must render with uncertainty language",
        PlayerRenderPermission::SourcedRumor => "must render with source/rumor framing",
        PlayerRenderPermission::BeliefOnly => "must render as a holder belief, not world fact",
        PlayerRenderPermission::BlockedHiddenTruth => "must not render to player-visible surfaces",
    }
}

#[must_use]
pub fn visible_knowledge_text_is_qualified(tier: KnowledgeTier, text: &str) -> bool {
    match player_render_permission(tier) {
        PlayerRenderPermission::DirectFact => true,
        PlayerRenderPermission::UncertainInference => contains_any(
            text,
            &[
                "추정",
                "가능성",
                "듯",
                "아마",
                "확실하지",
                "inferred",
                "seems",
                "may",
                "might",
                "uncertain",
            ],
        ),
        PlayerRenderPermission::SourcedRumor => contains_any(
            text,
            &[
                "소문", "전해", "들었", "출처", "rumor", "heard", "reported", "source",
            ],
        ),
        PlayerRenderPermission::BeliefOnly => contains_any(
            text,
            &["믿", "오해", "착각", "belief", "believes", "mistaken"],
        ),
        PlayerRenderPermission::BlockedHiddenTruth => false,
    }
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    let lower = text.to_ascii_lowercase();
    needles
        .iter()
        .any(|needle| text.contains(needle) || lower.contains(&needle.to_ascii_lowercase()))
}

fn compile_holder_snapshots(claims: &[KnowledgeClaim]) -> Vec<KnowledgeHolderSnapshot> {
    let mut holders = BTreeMap::<String, KnowledgeHolderSnapshot>::new();
    for claim in claims {
        let snapshot =
            holders
                .entry(claim.holder_ref.clone())
                .or_insert_with(|| KnowledgeHolderSnapshot {
                    holder_ref: claim.holder_ref.clone(),
                    holder_kind: holder_kind_for_ref(claim.holder_ref.as_str()),
                    claim_count: 0,
                    hidden_claim_count: 0,
                    observed_claim_count: 0,
                    inferred_claim_count: 0,
                    rumor_claim_count: 0,
                    false_belief_count: 0,
                    contradicted_claim_count: 0,
                });
        snapshot.claim_count += 1;
        match claim.tier {
            KnowledgeTier::WorldTrueHidden => snapshot.hidden_claim_count += 1,
            KnowledgeTier::PlayerObserved => snapshot.observed_claim_count += 1,
            KnowledgeTier::PlayerInferred => snapshot.inferred_claim_count += 1,
            KnowledgeTier::Rumor => snapshot.rumor_claim_count += 1,
            KnowledgeTier::FalseBelief => snapshot.false_belief_count += 1,
            KnowledgeTier::Contradicted => snapshot.contradicted_claim_count += 1,
        }
    }
    holders.into_values().collect()
}

fn compile_knowledge_conflicts(claims: &[KnowledgeClaim]) -> Vec<KnowledgeConflict> {
    let mut by_claim_id = BTreeMap::<String, Vec<&KnowledgeClaim>>::new();
    for claim in claims {
        by_claim_id
            .entry(claim.claim_id.clone())
            .or_default()
            .push(claim);
    }

    by_claim_id
        .into_iter()
        .filter_map(|(claim_id, grouped_claims)| {
            let holder_refs = grouped_claims
                .iter()
                .map(|claim| claim.holder_ref.clone())
                .collect::<BTreeSet<_>>();
            if holder_refs.len() < 2 {
                return None;
            }

            let propositions = grouped_claims
                .iter()
                .map(|claim| claim.proposition.clone())
                .collect::<BTreeSet<_>>();
            let truth_statuses = grouped_claims
                .iter()
                .map(|claim| claim.truth_status)
                .collect::<BTreeSet<_>>();
            let tiers = grouped_claims
                .iter()
                .map(|claim| claim.tier)
                .collect::<BTreeSet<_>>();

            let mut conflict_kinds = Vec::new();
            if propositions.len() > 1 {
                conflict_kinds.push(KnowledgeConflictKind::PropositionDisagreement);
            }
            if truth_statuses.len() > 1 {
                conflict_kinds.push(KnowledgeConflictKind::TruthStatusDisagreement);
            }
            if tiers.len() > 1 {
                conflict_kinds.push(KnowledgeConflictKind::TierDisagreement);
            }
            if conflict_kinds.is_empty() {
                return None;
            }

            Some(KnowledgeConflict {
                claim_id,
                holder_refs: holder_refs.into_iter().collect(),
                conflict_kinds,
                propositions: propositions.into_iter().collect(),
                truth_statuses: truth_statuses.into_iter().collect(),
                tiers: tiers.into_iter().collect(),
            })
        })
        .collect()
}

fn holder_kind_for_ref(holder_ref: &str) -> KnowledgeHolderKind {
    if holder_ref == "player" || holder_ref.starts_with("player:") {
        KnowledgeHolderKind::Player
    } else if holder_ref == "world" || holder_ref.starts_with("world:") {
        KnowledgeHolderKind::World
    } else if holder_ref.starts_with("npc:") || holder_ref.starts_with("char:") {
        KnowledgeHolderKind::Npc
    } else if holder_ref == "system" || holder_ref.starts_with("system:") {
        KnowledgeHolderKind::System
    } else {
        KnowledgeHolderKind::Other
    }
}

fn append_claim_event(
    world_dir: &Path,
    input: KnowledgeClaimEventInput,
    event_kind: KnowledgeEventKind,
    tier: KnowledgeTier,
    truth_status: TruthStatus,
) -> Result<KnowledgeEventRecord> {
    validate_claim_input(&input)?;
    append_knowledge_event(world_dir, |event_id, recorded_at| KnowledgeEventRecord {
        schema_version: KNOWLEDGE_EVENT_SCHEMA_VERSION.to_owned(),
        event_id,
        world_id: input.world_id,
        turn_id: input.turn_id,
        claim_id: input.claim_id,
        event_kind,
        holder_ref: input.holder_ref,
        from_holder_ref: None,
        tier,
        truth_status,
        proposition: input.proposition,
        evidence_refs: input.evidence_refs,
        recorded_at,
    })
}

fn append_knowledge_event(
    world_dir: &Path,
    build: impl FnOnce(String, String) -> KnowledgeEventRecord,
) -> Result<KnowledgeEventRecord> {
    let existing = load_knowledge_events(world_dir)?;
    let event_id = format!("knowledge_event:{:06}", existing.len());
    let event = build(event_id, Utc::now().to_rfc3339());
    validate_event(event.world_id.as_str(), &event)?;
    append_jsonl(&world_dir.join(KNOWLEDGE_EVENTS_FILENAME), &event)?;
    Ok(event)
}

fn validate_claim_input(input: &KnowledgeClaimEventInput) -> Result<()> {
    if input.world_id.trim().is_empty() {
        bail!("knowledge claim world_id must not be empty");
    }
    if input.turn_id.trim().is_empty() {
        bail!("knowledge claim turn_id must not be empty");
    }
    if input.claim_id.trim().is_empty() {
        bail!("knowledge claim claim_id must not be empty");
    }
    if input.holder_ref.trim().is_empty() {
        bail!("knowledge claim holder_ref must not be empty");
    }
    if input.proposition.trim().is_empty() {
        bail!("knowledge claim proposition must not be empty");
    }
    Ok(())
}

fn validate_transfer_input(input: &KnowledgeTransferInput) -> Result<()> {
    validate_claim_input(&KnowledgeClaimEventInput {
        world_id: input.world_id.clone(),
        turn_id: input.turn_id.clone(),
        claim_id: input.claim_id.clone(),
        holder_ref: input.to_holder_ref.clone(),
        proposition: input.proposition.clone(),
        evidence_refs: input.evidence_refs.clone(),
    })?;
    if input.from_holder_ref.trim().is_empty() {
        bail!("knowledge transfer from_holder_ref must not be empty");
    }
    if input.from_holder_ref == input.to_holder_ref {
        bail!(
            "knowledge transfer requires distinct holders: holder_ref={}",
            input.to_holder_ref
        );
    }
    Ok(())
}

fn validate_event(expected_world_id: &str, event: &KnowledgeEventRecord) -> Result<()> {
    if event.schema_version != KNOWLEDGE_EVENT_SCHEMA_VERSION {
        bail!(
            "knowledge event schema_version mismatch: expected={}, actual={}, event_id={}",
            KNOWLEDGE_EVENT_SCHEMA_VERSION,
            event.schema_version,
            event.event_id
        );
    }
    if event.world_id != expected_world_id {
        bail!(
            "knowledge event world_id mismatch: expected={}, actual={}, event_id={}",
            expected_world_id,
            event.world_id,
            event.event_id
        );
    }
    if event.event_id.trim().is_empty() {
        bail!("knowledge event event_id must not be empty");
    }
    if event.turn_id.trim().is_empty() {
        bail!(
            "knowledge event turn_id must not be empty: event_id={}",
            event.event_id
        );
    }
    if event.claim_id.trim().is_empty() {
        bail!(
            "knowledge event claim_id must not be empty: event_id={}",
            event.event_id
        );
    }
    if event.holder_ref.trim().is_empty() {
        bail!(
            "knowledge event holder_ref must not be empty: event_id={}",
            event.event_id
        );
    }
    if event.proposition.trim().is_empty() {
        bail!(
            "knowledge event proposition must not be empty: event_id={}",
            event.event_id
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        KnowledgeClaimEventInput, KnowledgeConflictKind, KnowledgeHolderKind, KnowledgeTier,
        KnowledgeTransferInput, PlayerRenderPermission, TruthStatus,
        can_render_knowledge_tier_to_player, confirm_claim, contradict_claim, false_belief_claim,
        hidden_truth_claim, infer_claim, load_knowledge_events, observe_claim,
        player_render_permission, rebuild_knowledge_ledger, render_rule_for_player,
        replay_knowledge_state, rumor_claim, transfer_claim_between_holders,
    };
    use tempfile::tempdir;

    #[test]
    fn hidden_truth_is_not_player_renderable() {
        assert!(!can_render_knowledge_tier_to_player(
            KnowledgeTier::WorldTrueHidden
        ));
        assert_eq!(
            player_render_permission(KnowledgeTier::WorldTrueHidden),
            PlayerRenderPermission::BlockedHiddenTruth
        );
    }

    #[test]
    fn inferred_and_rumor_tiers_require_qualified_rendering() {
        assert_eq!(
            player_render_permission(KnowledgeTier::PlayerInferred),
            PlayerRenderPermission::UncertainInference
        );
        assert_eq!(
            player_render_permission(KnowledgeTier::Rumor),
            PlayerRenderPermission::SourcedRumor
        );
        assert!(render_rule_for_player(KnowledgeTier::Rumor).contains("source"));
        assert!(super::visible_knowledge_text_is_qualified(
            KnowledgeTier::PlayerInferred,
            "북문 뒤쪽에 누군가 있을 가능성이 있다"
        ));
        assert!(!super::visible_knowledge_text_is_qualified(
            KnowledgeTier::PlayerInferred,
            "북문 뒤쪽에 암살자가 있다"
        ));
    }

    #[test]
    fn persists_and_replays_observed_inferred_and_rumor_claims() -> anyhow::Result<()> {
        let temp = tempdir()?;
        observe_claim(
            temp.path(),
            claim_input("claim:gate", "player", "문이 젖어 있다"),
        )?;
        infer_claim(
            temp.path(),
            claim_input("claim:latch", "player", "안쪽 걸쇠가 휘었을 가능성이 있다"),
        )?;
        rumor_claim(
            temp.path(),
            claim_input("claim:guard", "npc:guard", "폐문 시간이 앞당겨졌다는 소문"),
        )?;

        let events = load_knowledge_events(temp.path())?;
        let state = replay_knowledge_state("stw_knowledge", &events)?;

        assert_eq!(events.len(), 3);
        assert_eq!(state.claims.len(), 3);
        assert_eq!(state.holders.len(), 2);
        let Some(player_holder) = state
            .holders
            .iter()
            .find(|holder| holder.holder_ref == "player")
        else {
            anyhow::bail!("player holder snapshot should replay");
        };
        assert_eq!(player_holder.holder_kind, KnowledgeHolderKind::Player);
        assert_eq!(player_holder.claim_count, 2);
        assert_eq!(player_holder.observed_claim_count, 1);
        assert_eq!(player_holder.inferred_claim_count, 1);
        assert!(state.claims.iter().any(|claim| {
            claim.claim_id == "claim:latch" && claim.tier == KnowledgeTier::PlayerInferred
        }));
        Ok(())
    }

    #[test]
    fn confirm_and_contradict_update_latest_holder_claim() -> anyhow::Result<()> {
        let temp = tempdir()?;
        infer_claim(
            temp.path(),
            claim_input(
                "claim:north_gate",
                "player",
                "북문 뒤에 누군가 있을 가능성이 있다",
            ),
        )?;
        confirm_claim(
            temp.path(),
            claim_input("claim:north_gate", "player", "북문 뒤에 경비병이 있다"),
        )?;
        contradict_claim(
            temp.path(),
            claim_input(
                "claim:north_gate",
                "player",
                "북문 뒤의 기척은 암살자가 아니다",
            ),
        )?;

        let state = rebuild_knowledge_ledger(temp.path(), "stw_knowledge")?;
        let Some(claim) = state
            .claims
            .iter()
            .find(|claim| claim.claim_id == "claim:north_gate")
        else {
            anyhow::bail!("claim should replay");
        };

        assert_eq!(claim.tier, KnowledgeTier::Contradicted);
        assert_eq!(claim.truth_status, TruthStatus::Contested);
        assert!(temp.path().join(super::KNOWLEDGE_LEDGER_FILENAME).is_file());
        Ok(())
    }

    #[test]
    fn transfers_claim_between_distinct_holders() -> anyhow::Result<()> {
        let temp = tempdir()?;
        transfer_claim_between_holders(
            temp.path(),
            KnowledgeTransferInput {
                world_id: "stw_knowledge".to_owned(),
                turn_id: "turn_0002".to_owned(),
                claim_id: "claim:rumor".to_owned(),
                from_holder_ref: "npc:guard".to_owned(),
                to_holder_ref: "player".to_owned(),
                tier: KnowledgeTier::Rumor,
                truth_status: TruthStatus::Rumored,
                proposition: "서문 쪽에 검문이 느슨하다는 소문".to_owned(),
                evidence_refs: vec!["dialogue:guard".to_owned()],
            },
        )?;

        let events = load_knowledge_events(temp.path())?;
        let state = replay_knowledge_state("stw_knowledge", &events)?;

        assert_eq!(events[0].from_holder_ref.as_deref(), Some("npc:guard"));
        assert_eq!(state.claims[0].holder_ref, "player");
        assert_eq!(state.claims[0].tier, KnowledgeTier::Rumor);
        Ok(())
    }

    #[test]
    fn replay_reports_holder_conflicts_without_collapsing_beliefs() -> anyhow::Result<()> {
        let temp = tempdir()?;
        hidden_truth_claim(
            temp.path(),
            claim_input("claim:ambush", "world", "암살자는 북문 뒤에 숨어 있다"),
        )?;
        false_belief_claim(
            temp.path(),
            claim_input("claim:ambush", "player", "암살자는 서문 쪽에 있을 것이다"),
        )?;
        rumor_claim(
            temp.path(),
            claim_input("claim:ambush", "npc:guard", "북문 뒤에 누군가 있다는 소문"),
        )?;

        let events = load_knowledge_events(temp.path())?;
        let state = replay_knowledge_state("stw_knowledge", &events)?;

        assert_eq!(state.claims.len(), 3);
        assert_eq!(state.holders.len(), 3);
        assert!(state.holders.iter().any(|holder| {
            holder.holder_ref == "npc:guard" && holder.holder_kind == KnowledgeHolderKind::Npc
        }));
        let Some(conflict) = state
            .conflicts
            .iter()
            .find(|conflict| conflict.claim_id == "claim:ambush")
        else {
            anyhow::bail!("holder conflict should replay");
        };
        assert_eq!(
            conflict.holder_refs,
            vec![
                "npc:guard".to_owned(),
                "player".to_owned(),
                "world".to_owned()
            ]
        );
        assert!(
            conflict
                .conflict_kinds
                .contains(&KnowledgeConflictKind::PropositionDisagreement)
        );
        assert!(
            conflict
                .conflict_kinds
                .contains(&KnowledgeConflictKind::TruthStatusDisagreement)
        );
        assert!(
            conflict
                .conflict_kinds
                .contains(&KnowledgeConflictKind::TierDisagreement)
        );
        Ok(())
    }

    fn claim_input(
        claim_id: &str,
        holder_ref: &str,
        proposition: &str,
    ) -> KnowledgeClaimEventInput {
        KnowledgeClaimEventInput {
            world_id: "stw_knowledge".to_owned(),
            turn_id: "turn_0001".to_owned(),
            claim_id: claim_id.to_owned(),
            holder_ref: holder_ref.to_owned(),
            proposition: proposition.to_owned(),
            evidence_refs: vec!["current_turn".to_owned()],
        }
    }
}
