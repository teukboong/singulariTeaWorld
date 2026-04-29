#![allow(clippy::missing_errors_doc)]

use crate::character_text_design::{CharacterTextDesign, CharacterTextDesignPacket};
use crate::relationship_graph::{RelationshipEdge, RelationshipGraphPacket};
use crate::store::{append_jsonl, read_json, write_json};
use crate::turn_retrieval_controller::{
    TurnRetrievalControllerPacket, TurnRetrievalCueReason, TurnRetrievalTargetKind,
    TurnRetrievalVisibility,
};
use crate::world_lore::{WorldLoreEntry, WorldLorePacket};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const CONTEXT_CAPSULE_INDEX_SCHEMA_VERSION: &str = "singulari.context_capsule_index.v1";
pub const CONTEXT_CAPSULE_INDEX_ENTRY_SCHEMA_VERSION: &str =
    "singulari.context_capsule_index_entry.v1";
pub const CONTEXT_CAPSULE_SCHEMA_VERSION: &str = "singulari.context_capsule.v1";
pub const CONTEXT_CAPSULE_SELECTION_SCHEMA_VERSION: &str = "singulari.context_capsule_selection.v1";
pub const CONTEXT_CAPSULE_SELECTION_EVENT_SCHEMA_VERSION: &str =
    "singulari.context_capsule_selection_event.v1";
pub const CONTEXT_CAPSULE_DIR: &str = "context_capsules";
pub const CONTEXT_CAPSULE_INDEX_FILENAME: &str = "capsule_index.json";
pub const CONTEXT_CAPSULE_SELECTION_EVENTS_FILENAME: &str =
    "context_capsule_selection_events.jsonl";

const SELECTED_CONTEXT_CAPSULE_BUDGET: usize = 8;
const RECENT_SELECTION_EVENT_WINDOW: usize = 6;
const STALE_FALLBACK_REUSE_LIMIT: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextCapsuleIndex {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub entries: Vec<ContextCapsuleIndexEntry>,
    pub compiler_policy: ContextCapsulePolicy,
}

impl Default for ContextCapsuleIndex {
    fn default() -> Self {
        Self {
            schema_version: CONTEXT_CAPSULE_INDEX_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            turn_id: String::new(),
            entries: Vec::new(),
            compiler_policy: ContextCapsulePolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextCapsuleIndexEntry {
    pub schema_version: String,
    pub world_id: String,
    pub capsule_id: String,
    pub kind: ContextCapsuleKind,
    pub visibility: ContextCapsuleVisibility,
    pub summary: String,
    #[serde(default)]
    pub triggers: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<ContextCapsuleEvidenceRef>,
    pub content_ref: String,
    pub token_estimate: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_turn: Option<String>,
    pub recent_use_count: u32,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextCapsule {
    pub schema_version: String,
    pub world_id: String,
    pub capsule_id: String,
    pub kind: ContextCapsuleKind,
    pub visibility: ContextCapsuleVisibility,
    pub payload: Value,
    #[serde(default)]
    pub use_rules: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<ContextCapsuleEvidenceRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextCapsuleSelection {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub selected_capsules: Vec<SelectedContextCapsule>,
    #[serde(default)]
    pub rejected_capsules: Vec<RejectedContextCapsule>,
    pub budget_report: ContextCapsuleBudgetReport,
    pub compiler_policy: ContextCapsulePolicy,
}

impl Default for ContextCapsuleSelection {
    fn default() -> Self {
        Self {
            schema_version: CONTEXT_CAPSULE_SELECTION_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            turn_id: String::new(),
            selected_capsules: Vec::new(),
            rejected_capsules: Vec::new(),
            budget_report: ContextCapsuleBudgetReport::default(),
            compiler_policy: ContextCapsulePolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SelectedContextCapsule {
    pub capsule_id: String,
    pub kind: ContextCapsuleKind,
    pub visibility: ContextCapsuleVisibility,
    pub reason: ContextCapsuleSelectionReason,
    pub score: f32,
    pub payload: Value,
    #[serde(default)]
    pub evidence_refs: Vec<ContextCapsuleEvidenceRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RejectedContextCapsule {
    pub capsule_id: String,
    pub kind: ContextCapsuleKind,
    pub reason: ContextCapsuleRejectReason,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextCapsuleBudgetReport {
    pub capsule_index_entries_seen: usize,
    pub capsules_loaded: usize,
    pub capsules_rejected: usize,
    pub estimated_tokens: usize,
    pub selected_limit: usize,
}

impl Default for ContextCapsuleBudgetReport {
    fn default() -> Self {
        Self {
            capsule_index_entries_seen: 0,
            capsules_loaded: 0,
            capsules_rejected: 0,
            estimated_tokens: 0,
            selected_limit: SELECTED_CONTEXT_CAPSULE_BUDGET,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextCapsuleSelectionEvent {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub event_id: String,
    #[serde(default)]
    pub selected_capsule_ids: Vec<String>,
    #[serde(default)]
    pub rejected_capsule_ids: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ContextCapsuleKind {
    WorldLore,
    RelationshipGraph,
    CharacterTextDesign,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextCapsuleVisibility {
    PlayerVisible,
    InferredVisible,
    Private,
    Hidden,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextCapsuleSelectionReason {
    DirectPlayerInputTrigger,
    CurrentGoalMatch,
    RoleStanceMatch,
    ScenePressureSource,
    ActiveProcessSource,
    ActiveProjectionFallback,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextCapsuleRejectReason {
    VisibilityBlocked,
    BudgetExceeded,
    ZeroScore,
    RepeatedWithoutEffect,
    SeedLeakageRisk,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextCapsuleEvidenceRef {
    pub source: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextCapsulePolicy {
    pub source: String,
    pub selected_capsule_budget: usize,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for ContextCapsulePolicy {
    fn default() -> Self {
        Self {
            source: "compiled_from_materialized_world_projections_v1".to_owned(),
            selected_capsule_budget: SELECTED_CONTEXT_CAPSULE_BUDGET,
            use_rules: vec![
                "Context capsules are prompt transport units, not source-of-truth state."
                    .to_owned(),
                "Capsule bodies must be rebuilt from typed projections, never invented as fallback summaries.".to_owned(),
                "Visible prompt selection may load only player-visible or inferred-visible capsules.".to_owned(),
            ],
        }
    }
}

pub struct ContextCapsuleBuildInput<'a> {
    pub world_dir: &'a Path,
    pub world_id: &'a str,
    pub turn_id: &'a str,
    pub active_world_lore: &'a WorldLorePacket,
    pub active_relationship_graph: &'a RelationshipGraphPacket,
    pub active_character_text_design: &'a CharacterTextDesignPacket,
}

pub struct ContextCapsuleSelectionInput<'a> {
    pub world_dir: &'a Path,
    pub world_id: &'a str,
    pub turn_id: &'a str,
    pub player_input: &'a str,
    pub retrieval_controller: &'a TurnRetrievalControllerPacket,
}

pub fn rebuild_context_capsule_registry(
    input: &ContextCapsuleBuildInput<'_>,
) -> Result<ContextCapsuleIndex> {
    let root = capsule_root(input.world_dir);
    fs::create_dir_all(&root)
        .with_context(|| format!("failed to create context capsule root: {}", root.display()))?;
    let mut entries = Vec::new();
    for entry in &input.active_world_lore.entries {
        entries.push(write_world_lore_capsule(input, entry)?);
    }
    for edge in &input.active_relationship_graph.active_edges {
        entries.push(write_relationship_capsule(input, edge)?);
    }
    for design in &input.active_character_text_design.active_designs {
        entries.push(write_character_text_capsule(input, design)?);
    }
    let index = ContextCapsuleIndex {
        schema_version: CONTEXT_CAPSULE_INDEX_SCHEMA_VERSION.to_owned(),
        world_id: input.world_id.to_owned(),
        turn_id: input.turn_id.to_owned(),
        entries,
        compiler_policy: ContextCapsulePolicy::default(),
    };
    write_json(&root.join(CONTEXT_CAPSULE_INDEX_FILENAME), &index)?;
    Ok(index)
}

pub fn select_context_capsules(
    input: &ContextCapsuleSelectionInput<'_>,
) -> Result<ContextCapsuleSelection> {
    let index = load_context_capsule_index(input.world_dir, input.world_id, input.turn_id)?;
    let recent_usage = recent_capsule_usage(input.world_dir)?;
    let mut scored = Vec::new();
    let mut rejected_capsules = Vec::new();
    for entry in &index.entries {
        if !is_visible_for_prompt(entry.visibility) {
            rejected_capsules.push(rejected(
                entry,
                ContextCapsuleRejectReason::VisibilityBlocked,
            ));
            continue;
        }
        if seed_leakage_risk(entry) {
            rejected_capsules.push(rejected(entry, ContextCapsuleRejectReason::SeedLeakageRisk));
            continue;
        }
        let candidate = score_capsule_entry(entry, input.player_input, input.retrieval_controller);
        if is_stale_fallback(&candidate, &recent_usage) {
            rejected_capsules.push(rejected(
                entry,
                ContextCapsuleRejectReason::RepeatedWithoutEffect,
            ));
            continue;
        }
        scored.push(candidate);
    }
    scored.sort_by(compare_scored_capsules);

    let mut selected_capsules = Vec::new();
    for candidate in scored {
        if selected_capsules.len() >= SELECTED_CONTEXT_CAPSULE_BUDGET {
            rejected_capsules.push(RejectedContextCapsule {
                capsule_id: candidate.entry.capsule_id.clone(),
                kind: candidate.entry.kind,
                reason: ContextCapsuleRejectReason::BudgetExceeded,
            });
            continue;
        }
        let capsule = load_context_capsule_body(input.world_dir, candidate.entry)?;
        selected_capsules.push(SelectedContextCapsule {
            capsule_id: candidate.entry.capsule_id.clone(),
            kind: candidate.entry.kind,
            visibility: candidate.entry.visibility,
            reason: candidate.reason,
            score: candidate.score,
            payload: capsule.payload,
            evidence_refs: capsule.evidence_refs,
        });
    }

    let estimated_tokens = selected_capsules
        .iter()
        .map(|capsule| {
            index
                .entries
                .iter()
                .find(|entry| entry.capsule_id == capsule.capsule_id)
                .map_or(0, |entry| entry.token_estimate)
        })
        .sum();
    let selection = ContextCapsuleSelection {
        schema_version: CONTEXT_CAPSULE_SELECTION_SCHEMA_VERSION.to_owned(),
        world_id: input.world_id.to_owned(),
        turn_id: input.turn_id.to_owned(),
        budget_report: ContextCapsuleBudgetReport {
            capsule_index_entries_seen: index.entries.len(),
            capsules_loaded: selected_capsules.len(),
            capsules_rejected: rejected_capsules.len(),
            estimated_tokens,
            selected_limit: SELECTED_CONTEXT_CAPSULE_BUDGET,
        },
        selected_capsules,
        rejected_capsules,
        compiler_policy: ContextCapsulePolicy::default(),
    };
    append_selection_event_once(input.world_dir, &selection)?;
    Ok(selection)
}

fn rejected(
    entry: &ContextCapsuleIndexEntry,
    reason: ContextCapsuleRejectReason,
) -> RejectedContextCapsule {
    RejectedContextCapsule {
        capsule_id: entry.capsule_id.clone(),
        kind: entry.kind,
        reason,
    }
}

fn load_context_capsule_index(
    world_dir: &Path,
    world_id: &str,
    turn_id: &str,
) -> Result<ContextCapsuleIndex> {
    let path = capsule_root(world_dir).join(CONTEXT_CAPSULE_INDEX_FILENAME);
    if !path.is_file() {
        bail!(
            "context capsule index missing: world_id={world_id}, turn_id={turn_id}, path={}",
            path.display()
        );
    }
    let index: ContextCapsuleIndex = read_json(&path)?;
    if index.schema_version != CONTEXT_CAPSULE_INDEX_SCHEMA_VERSION {
        bail!(
            "context capsule index schema mismatch: expected={}, actual={}, path={}",
            CONTEXT_CAPSULE_INDEX_SCHEMA_VERSION,
            index.schema_version,
            path.display()
        );
    }
    if index.world_id != world_id || index.turn_id != turn_id {
        bail!(
            "context capsule index target mismatch: expected_world={world_id}, actual_world={}, expected_turn={turn_id}, actual_turn={}, path={}",
            index.world_id,
            index.turn_id,
            path.display()
        );
    }
    Ok(index)
}

fn load_context_capsule_body(
    world_dir: &Path,
    entry: &ContextCapsuleIndexEntry,
) -> Result<ContextCapsule> {
    validate_index_entry(entry)?;
    let path = world_dir.join(&entry.content_ref);
    let capsule: ContextCapsule = read_json(&path)
        .with_context(|| format!("context capsule body read failed: {}", path.display()))?;
    if capsule.schema_version != CONTEXT_CAPSULE_SCHEMA_VERSION {
        bail!(
            "context capsule body schema mismatch: capsule_id={}, expected={}, actual={}",
            entry.capsule_id,
            CONTEXT_CAPSULE_SCHEMA_VERSION,
            capsule.schema_version
        );
    }
    if capsule.capsule_id != entry.capsule_id {
        bail!(
            "context capsule body id mismatch: index={}, body={}",
            entry.capsule_id,
            capsule.capsule_id
        );
    }
    if capsule.world_id != entry.world_id || capsule.kind != entry.kind {
        bail!(
            "context capsule body target mismatch: capsule_id={}, index_world={}, body_world={}, index_kind={:?}, body_kind={:?}",
            entry.capsule_id,
            entry.world_id,
            capsule.world_id,
            entry.kind,
            capsule.kind
        );
    }
    Ok(capsule)
}

fn validate_index_entry(entry: &ContextCapsuleIndexEntry) -> Result<()> {
    if entry.schema_version != CONTEXT_CAPSULE_INDEX_ENTRY_SCHEMA_VERSION {
        bail!(
            "context capsule index entry schema mismatch: capsule_id={}, expected={}, actual={}",
            entry.capsule_id,
            CONTEXT_CAPSULE_INDEX_ENTRY_SCHEMA_VERSION,
            entry.schema_version
        );
    }
    let content_ref = Path::new(entry.content_ref.as_str());
    if content_ref.is_absolute()
        || content_ref
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!(
            "context capsule content_ref escapes world dir: capsule_id={}, content_ref={}",
            entry.capsule_id,
            entry.content_ref
        );
    }
    if !content_ref.starts_with(CONTEXT_CAPSULE_DIR) {
        bail!(
            "context capsule content_ref outside capsule root: capsule_id={}, content_ref={}",
            entry.capsule_id,
            entry.content_ref
        );
    }
    Ok(())
}

fn write_world_lore_capsule(
    input: &ContextCapsuleBuildInput<'_>,
    entry: &WorldLoreEntry,
) -> Result<ContextCapsuleIndexEntry> {
    let capsule_id = format!("world_lore:{}", sanitize_ref(entry.lore_id.as_str()));
    let evidence_refs = entry
        .source_refs
        .iter()
        .map(|id| ContextCapsuleEvidenceRef {
            source: "world_lore".to_owned(),
            id: id.clone(),
        })
        .collect::<Vec<_>>();
    let triggers = trigger_terms([entry.name.as_str(), entry.summary.as_str()]);
    let payload = json!({
        "name": entry.name,
        "domain": entry.domain,
        "summary": entry.summary,
        "mechanical_axis": entry.mechanical_axis,
    });
    write_capsule(
        input,
        CapsuleSeed {
            capsule_id,
            kind: ContextCapsuleKind::WorldLore,
            visibility: visibility_from_str(entry.visibility.as_str()),
            summary: entry.summary.clone(),
            triggers,
            evidence_refs,
            payload,
            use_rules: vec![
                "Use as player-visible world constraint only.".to_owned(),
                "Do not invent additional customs, institutions, or history from this capsule."
                    .to_owned(),
            ],
        },
    )
}

fn write_relationship_capsule(
    input: &ContextCapsuleBuildInput<'_>,
    edge: &RelationshipEdge,
) -> Result<ContextCapsuleIndexEntry> {
    let capsule_id = format!("relationship:{}", sanitize_ref(edge.edge_id.as_str()));
    let evidence_refs = edge
        .source_refs
        .iter()
        .map(|id| ContextCapsuleEvidenceRef {
            source: "relationship_graph".to_owned(),
            id: id.clone(),
        })
        .collect::<Vec<_>>();
    let triggers = trigger_terms([
        edge.source_entity_id.as_str(),
        edge.target_entity_id.as_str(),
        edge.stance.as_str(),
        edge.visible_summary.as_str(),
    ]);
    let payload = json!({
        "source_entity_id": edge.source_entity_id,
        "target_entity_id": edge.target_entity_id,
        "stance": edge.stance,
        "visible_summary": edge.visible_summary,
        "voice_effects": edge.voice_effects,
    });
    write_capsule(
        input,
        CapsuleSeed {
            capsule_id,
            kind: ContextCapsuleKind::RelationshipGraph,
            visibility: visibility_from_str(edge.visibility.as_str()),
            summary: edge.visible_summary.clone(),
            triggers,
            evidence_refs,
            payload,
            use_rules: vec![
                "Use as social stance and dialogue-distance pressure.".to_owned(),
                "Do not infer hidden motives or personality traits.".to_owned(),
            ],
        },
    )
}

fn write_character_text_capsule(
    input: &ContextCapsuleBuildInput<'_>,
    design: &CharacterTextDesign,
) -> Result<ContextCapsuleIndexEntry> {
    let capsule_id = format!("character_text:{}", sanitize_ref(design.entity_id.as_str()));
    let evidence_refs = design
        .source_refs
        .iter()
        .map(|id| ContextCapsuleEvidenceRef {
            source: "character_text_design".to_owned(),
            id: id.clone(),
        })
        .collect::<Vec<_>>();
    let triggers = trigger_terms([
        design.entity_id.as_str(),
        design.visible_name.as_str(),
        design.role.as_str(),
    ]);
    let summary = format!("{} speech design: {}", design.visible_name, design.role);
    let payload = json!({
        "entity_id": design.entity_id,
        "visible_name": design.visible_name,
        "role": design.role,
        "speech": design.speech,
        "endings": design.endings,
        "tone": design.tone,
        "gestures": design.gestures,
        "habits": design.habits,
        "drift": design.drift,
    });
    write_capsule(
        input,
        CapsuleSeed {
            capsule_id,
            kind: ContextCapsuleKind::CharacterTextDesign,
            visibility: visibility_from_str(design.visibility.as_str()),
            summary,
            triggers,
            evidence_refs,
            payload,
            use_rules: vec![
                "Use only when this character speaks, acts, or is directly perceived.".to_owned(),
                "Do not let character voice override global prose style.".to_owned(),
            ],
        },
    )
}

struct CapsuleSeed {
    capsule_id: String,
    kind: ContextCapsuleKind,
    visibility: ContextCapsuleVisibility,
    summary: String,
    triggers: Vec<String>,
    evidence_refs: Vec<ContextCapsuleEvidenceRef>,
    payload: Value,
    use_rules: Vec<String>,
}

fn write_capsule(
    input: &ContextCapsuleBuildInput<'_>,
    seed: CapsuleSeed,
) -> Result<ContextCapsuleIndexEntry> {
    let relative = capsule_relative_path(seed.kind, seed.capsule_id.as_str());
    let path = input.world_dir.join(&relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create capsule dir: {}", parent.display()))?;
    }
    let capsule = ContextCapsule {
        schema_version: CONTEXT_CAPSULE_SCHEMA_VERSION.to_owned(),
        world_id: input.world_id.to_owned(),
        capsule_id: seed.capsule_id.clone(),
        kind: seed.kind,
        visibility: seed.visibility,
        payload: seed.payload,
        use_rules: seed.use_rules,
        evidence_refs: seed.evidence_refs.clone(),
    };
    write_json(&path, &capsule)?;
    Ok(ContextCapsuleIndexEntry {
        schema_version: CONTEXT_CAPSULE_INDEX_ENTRY_SCHEMA_VERSION.to_owned(),
        world_id: input.world_id.to_owned(),
        capsule_id: seed.capsule_id,
        kind: seed.kind,
        visibility: seed.visibility,
        summary: seed.summary.clone(),
        triggers: seed.triggers,
        evidence_refs: seed.evidence_refs,
        content_ref: relative.to_string_lossy().to_string(),
        token_estimate: token_estimate(seed.summary.as_str()),
        last_used_turn: None,
        recent_use_count: 0,
        updated_at: Utc::now().to_rfc3339(),
    })
}

struct ScoredCapsule<'a> {
    entry: &'a ContextCapsuleIndexEntry,
    reason: ContextCapsuleSelectionReason,
    score: f32,
}

fn compare_scored_capsules(
    left: &ScoredCapsule<'_>,
    right: &ScoredCapsule<'_>,
) -> std::cmp::Ordering {
    right
        .score
        .total_cmp(&left.score)
        .then_with(|| left.entry.kind.cmp(&right.entry.kind))
        .then_with(|| left.entry.capsule_id.cmp(&right.entry.capsule_id))
}

fn score_capsule_entry<'a>(
    entry: &'a ContextCapsuleIndexEntry,
    player_input: &str,
    retrieval_controller: &TurnRetrievalControllerPacket,
) -> ScoredCapsule<'a> {
    let haystack = player_input.to_lowercase();
    if !haystack.trim().is_empty()
        && entry
            .triggers
            .iter()
            .any(|trigger| !trigger.is_empty() && haystack.contains(trigger))
    {
        return ScoredCapsule {
            entry,
            reason: ContextCapsuleSelectionReason::DirectPlayerInputTrigger,
            score: 1.0,
        };
    }
    if let Some(cue_score) = score_entry_from_retrieval_cues(entry, retrieval_controller) {
        return ScoredCapsule {
            entry,
            reason: cue_score.reason,
            score: cue_score.score,
        };
    }
    ScoredCapsule {
        entry,
        reason: ContextCapsuleSelectionReason::ActiveProjectionFallback,
        score: 0.35,
    }
}

fn is_visible_for_prompt(visibility: ContextCapsuleVisibility) -> bool {
    matches!(
        visibility,
        ContextCapsuleVisibility::PlayerVisible | ContextCapsuleVisibility::InferredVisible
    )
}

fn seed_leakage_risk(entry: &ContextCapsuleIndexEntry) -> bool {
    entry.evidence_refs.is_empty()
        && matches!(
            entry.kind,
            ContextCapsuleKind::WorldLore | ContextCapsuleKind::RelationshipGraph
        )
}

fn is_stale_fallback(candidate: &ScoredCapsule<'_>, recent_usage: &BTreeMap<String, u32>) -> bool {
    candidate.reason == ContextCapsuleSelectionReason::ActiveProjectionFallback
        && recent_usage
            .get(candidate.entry.capsule_id.as_str())
            .is_some_and(|count| *count >= STALE_FALLBACK_REUSE_LIMIT)
}

struct CueScore {
    reason: ContextCapsuleSelectionReason,
    score: f32,
}

fn score_entry_from_retrieval_cues(
    entry: &ContextCapsuleIndexEntry,
    retrieval_controller: &TurnRetrievalControllerPacket,
) -> Option<CueScore> {
    let mut best: Option<CueScore> = None;
    for cue in &retrieval_controller.retrieval_cues {
        if cue.visibility != TurnRetrievalVisibility::PlayerVisible {
            continue;
        }
        if !cue_target_matches_entry(cue.target_kinds.as_slice(), entry.kind) {
            continue;
        }
        let cue_terms = trigger_terms([cue.cue.as_str()]);
        let matched = entry.triggers.iter().any(|trigger| {
            cue_terms
                .iter()
                .any(|cue_term| trigger.contains(cue_term) || cue_term.contains(trigger))
        });
        if !matched {
            continue;
        }
        let candidate = CueScore {
            reason: selection_reason_from_cue(cue.reason),
            score: score_from_cue_reason(cue.reason),
        };
        if best
            .as_ref()
            .is_none_or(|existing| candidate.score > existing.score)
        {
            best = Some(candidate);
        }
    }
    best
}

fn cue_target_matches_entry(targets: &[TurnRetrievalTargetKind], kind: ContextCapsuleKind) -> bool {
    let expected = match kind {
        ContextCapsuleKind::WorldLore => TurnRetrievalTargetKind::WorldLore,
        ContextCapsuleKind::RelationshipGraph => TurnRetrievalTargetKind::RelationshipGraph,
        ContextCapsuleKind::CharacterTextDesign => TurnRetrievalTargetKind::CharacterTextDesign,
    };
    targets.contains(&expected)
}

fn selection_reason_from_cue(reason: TurnRetrievalCueReason) -> ContextCapsuleSelectionReason {
    match reason {
        TurnRetrievalCueReason::CurrentGoalMatch
        | TurnRetrievalCueReason::PlayerIntentContinuity
        | TurnRetrievalCueReason::SpatialAffordance
        | TurnRetrievalCueReason::BodyResourceGate => {
            ContextCapsuleSelectionReason::CurrentGoalMatch
        }
        TurnRetrievalCueReason::RoleStanceMatch => ContextCapsuleSelectionReason::RoleStanceMatch,
        TurnRetrievalCueReason::ScenePressureSource => {
            ContextCapsuleSelectionReason::ScenePressureSource
        }
        TurnRetrievalCueReason::ActiveProcessSource => {
            ContextCapsuleSelectionReason::ActiveProcessSource
        }
    }
}

fn score_from_cue_reason(reason: TurnRetrievalCueReason) -> f32 {
    match reason {
        TurnRetrievalCueReason::CurrentGoalMatch => 0.9,
        TurnRetrievalCueReason::ScenePressureSource => 0.82,
        TurnRetrievalCueReason::RoleStanceMatch => 0.78,
        TurnRetrievalCueReason::ActiveProcessSource => 0.74,
        TurnRetrievalCueReason::SpatialAffordance
        | TurnRetrievalCueReason::BodyResourceGate
        | TurnRetrievalCueReason::PlayerIntentContinuity => 0.7,
    }
}

fn selection_event(selection: &ContextCapsuleSelection) -> ContextCapsuleSelectionEvent {
    ContextCapsuleSelectionEvent {
        schema_version: CONTEXT_CAPSULE_SELECTION_EVENT_SCHEMA_VERSION.to_owned(),
        world_id: selection.world_id.clone(),
        turn_id: selection.turn_id.clone(),
        event_id: format!("context_capsule_selection_event:{}", selection.turn_id),
        selected_capsule_ids: selection
            .selected_capsules
            .iter()
            .map(|capsule| capsule.capsule_id.clone())
            .collect(),
        rejected_capsule_ids: selection
            .rejected_capsules
            .iter()
            .map(|capsule| capsule.capsule_id.clone())
            .collect(),
        created_at: Utc::now().to_rfc3339(),
    }
}

fn append_selection_event_once(
    world_dir: &Path,
    selection: &ContextCapsuleSelection,
) -> Result<()> {
    let path = world_dir.join(CONTEXT_CAPSULE_SELECTION_EVENTS_FILENAME);
    let event = selection_event(selection);
    if path.exists() {
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        for (index, line) in raw.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let existing: ContextCapsuleSelectionEvent =
                serde_json::from_str(line).with_context(|| {
                    format!(
                        "failed to parse {} line {} as ContextCapsuleSelectionEvent",
                        path.display(),
                        index + 1
                    )
                })?;
            if existing.event_id == event.event_id {
                return Ok(());
            }
        }
    }
    append_jsonl(&path, &event)
}

fn recent_capsule_usage(world_dir: &Path) -> Result<BTreeMap<String, u32>> {
    let path = world_dir.join(CONTEXT_CAPSULE_SELECTION_EVENTS_FILENAME);
    let mut usage = BTreeMap::new();
    if !path.exists() {
        return Ok(usage);
    }
    let raw =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut events = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        events.push(
            serde_json::from_str::<ContextCapsuleSelectionEvent>(line).with_context(|| {
                format!(
                    "failed to parse {} line {} as ContextCapsuleSelectionEvent",
                    path.display(),
                    index + 1
                )
            })?,
        );
    }
    let start = events.len().saturating_sub(RECENT_SELECTION_EVENT_WINDOW);
    for event in &events[start..] {
        for capsule_id in &event.selected_capsule_ids {
            *usage.entry(capsule_id.clone()).or_insert(0) += 1;
        }
    }
    Ok(usage)
}

fn capsule_root(world_dir: &Path) -> PathBuf {
    world_dir.join(CONTEXT_CAPSULE_DIR)
}

fn capsule_relative_path(kind: ContextCapsuleKind, capsule_id: &str) -> PathBuf {
    let subdir = match kind {
        ContextCapsuleKind::WorldLore => "world_lore",
        ContextCapsuleKind::RelationshipGraph => "relationship",
        ContextCapsuleKind::CharacterTextDesign => "character_text",
    };
    PathBuf::from(CONTEXT_CAPSULE_DIR)
        .join(subdir)
        .join(format!("{}.json", sanitize_ref(capsule_id)))
}

fn visibility_from_str(value: &str) -> ContextCapsuleVisibility {
    match value {
        "inferred_visible" => ContextCapsuleVisibility::InferredVisible,
        "private" => ContextCapsuleVisibility::Private,
        "hidden" => ContextCapsuleVisibility::Hidden,
        _ => ContextCapsuleVisibility::PlayerVisible,
    }
}

fn trigger_terms<'a>(parts: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut terms = parts
        .into_iter()
        .flat_map(|part| {
            part.split(|ch: char| {
                ch.is_whitespace()
                    || matches!(
                        ch,
                        ':' | ';' | ',' | '.' | '(' | ')' | '[' | ']' | '-' | '>'
                    )
            })
        })
        .map(|term| term.trim().to_lowercase())
        .filter(|term| term.chars().count() >= 2)
        .collect::<Vec<_>>();
    terms.sort();
    terms.dedup();
    terms
}

fn sanitize_ref(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn token_estimate(summary: &str) -> usize {
    summary.chars().count().div_ceil(3).max(24)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character_text_design::{
        CHARACTER_TEXT_DESIGN_PACKET_SCHEMA_VERSION, CharacterTextDesignPolicy,
    };
    use crate::relationship_graph::RELATIONSHIP_GRAPH_PACKET_SCHEMA_VERSION;
    use crate::turn_retrieval_controller::{
        TURN_RETRIEVAL_CONTROLLER_SCHEMA_VERSION, TURN_RETRIEVAL_CUE_SCHEMA_VERSION,
        TurnRetrievalCue, TurnRetrievalPolicy, TurnRetrievalVisibility,
    };
    use crate::world_lore::{
        WORLD_LORE_ENTRY_SCHEMA_VERSION, WORLD_LORE_PACKET_SCHEMA_VERSION, WorldLoreDomain,
        WorldLorePolicy,
    };

    #[test]
    fn selects_capsules_from_materialized_projection_index() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let lore = WorldLorePacket {
            schema_version: WORLD_LORE_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_capsule".to_owned(),
            turn_id: "turn_0002".to_owned(),
            entries: vec![WorldLoreEntry {
                schema_version: WORLD_LORE_ENTRY_SCHEMA_VERSION.to_owned(),
                lore_id: "gate_custom".to_owned(),
                domain: WorldLoreDomain::Customs,
                name: "Gate custom".to_owned(),
                summary: "Gate officials require a local token.".to_owned(),
                visibility: "player_visible".to_owned(),
                confidence: "confirmed".to_owned(),
                authority: "test".to_owned(),
                source_refs: vec!["world_lore_update:turn_0001:00".to_owned()],
                mechanical_axis: vec!["social_permission".to_owned()],
            }],
            compiler_policy: WorldLorePolicy::default(),
        };
        let relationships = RelationshipGraphPacket {
            schema_version: RELATIONSHIP_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_capsule".to_owned(),
            turn_id: "turn_0002".to_owned(),
            active_edges: Vec::new(),
            compiler_policy: crate::relationship_graph::RelationshipGraphPolicy::default(),
        };
        let character_text = CharacterTextDesignPacket {
            schema_version: CHARACTER_TEXT_DESIGN_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_capsule".to_owned(),
            active_designs: Vec::new(),
            compiler_policy: CharacterTextDesignPolicy::default(),
        };
        rebuild_context_capsule_registry(&ContextCapsuleBuildInput {
            world_dir: temp.path(),
            world_id: "stw_capsule",
            turn_id: "turn_0002",
            active_world_lore: &lore,
            active_relationship_graph: &relationships,
            active_character_text_design: &character_text,
        })?;

        let selection = select_context_capsules(&ContextCapsuleSelectionInput {
            world_dir: temp.path(),
            world_id: "stw_capsule",
            turn_id: "turn_0002",
            player_input: "gate token을 살핀다",
            retrieval_controller: &TurnRetrievalControllerPacket::default(),
        })?;

        assert_eq!(selection.selected_capsules.len(), 1);
        assert_eq!(
            selection.selected_capsules[0].reason,
            ContextCapsuleSelectionReason::DirectPlayerInputTrigger
        );
        assert_eq!(selection.budget_report.capsule_index_entries_seen, 1);
        Ok(())
    }

    #[test]
    fn selecting_same_turn_writes_one_selection_event() -> Result<()> {
        let temp = tempfile::tempdir()?;
        write_sample_registry(temp.path())?;

        for _ in 0..2 {
            select_context_capsules(&ContextCapsuleSelectionInput {
                world_dir: temp.path(),
                world_id: "stw_capsule",
                turn_id: "turn_0002",
                player_input: "gate token을 살핀다",
                retrieval_controller: &TurnRetrievalControllerPacket::default(),
            })?;
        }

        let raw = fs::read_to_string(temp.path().join(CONTEXT_CAPSULE_SELECTION_EVENTS_FILENAME))?;
        let event_count = raw.lines().filter(|line| !line.trim().is_empty()).count();
        assert_eq!(event_count, 1);
        Ok(())
    }

    #[test]
    fn rejects_index_for_wrong_turn() -> Result<()> {
        let temp = tempfile::tempdir()?;
        write_sample_registry(temp.path())?;

        let Err(error) = select_context_capsules(&ContextCapsuleSelectionInput {
            world_dir: temp.path(),
            world_id: "stw_capsule",
            turn_id: "turn_0003",
            player_input: "gate token을 살핀다",
            retrieval_controller: &TurnRetrievalControllerPacket::default(),
        }) else {
            bail!("expected context capsule turn mismatch to fail");
        };

        assert!(
            error
                .to_string()
                .contains("context capsule index target mismatch")
        );
        Ok(())
    }

    #[test]
    fn rejects_capsule_body_mismatch() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let index = write_sample_registry(temp.path())?;
        let first = &index.entries[0];
        let path = temp.path().join(&first.content_ref);
        let mut capsule: ContextCapsule = read_json(&path)?;
        capsule.capsule_id = "world_lore:other".to_owned();
        write_json(&path, &capsule)?;

        let Err(error) = select_context_capsules(&ContextCapsuleSelectionInput {
            world_dir: temp.path(),
            world_id: "stw_capsule",
            turn_id: "turn_0002",
            player_input: "gate token을 살핀다",
            retrieval_controller: &TurnRetrievalControllerPacket::default(),
        }) else {
            bail!("expected context capsule body mismatch to fail");
        };

        assert!(
            error
                .to_string()
                .contains("context capsule body id mismatch")
        );
        Ok(())
    }

    #[test]
    fn retrieval_controller_goal_boosts_matching_capsule() -> Result<()> {
        let temp = tempfile::tempdir()?;
        write_sample_registry(temp.path())?;
        let controller = TurnRetrievalControllerPacket {
            schema_version: TURN_RETRIEVAL_CONTROLLER_SCHEMA_VERSION.to_owned(),
            world_id: "stw_capsule".to_owned(),
            turn_id: "turn_0002".to_owned(),
            retrieval_cues: vec![TurnRetrievalCue {
                schema_version: TURN_RETRIEVAL_CUE_SCHEMA_VERSION.to_owned(),
                cue_id: "cue:turn_0002:gate".to_owned(),
                cue: "Gate access".to_owned(),
                reason: TurnRetrievalCueReason::CurrentGoalMatch,
                target_kinds: vec![TurnRetrievalTargetKind::WorldLore],
                visibility: TurnRetrievalVisibility::PlayerVisible,
                source_refs: vec!["plot_thread:gate".to_owned()],
            }],
            retrieval_policy: TurnRetrievalPolicy::default(),
            ..TurnRetrievalControllerPacket::default()
        };

        let selection = select_context_capsules(&ContextCapsuleSelectionInput {
            world_dir: temp.path(),
            world_id: "stw_capsule",
            turn_id: "turn_0002",
            player_input: "",
            retrieval_controller: &controller,
        })?;

        assert_eq!(
            selection.selected_capsules[0].reason,
            ContextCapsuleSelectionReason::CurrentGoalMatch
        );
        Ok(())
    }

    #[test]
    fn hidden_retrieval_cue_is_blocked_from_visible_capsule_selection() -> Result<()> {
        let temp = tempfile::tempdir()?;
        write_sample_registry(temp.path())?;
        let controller = TurnRetrievalControllerPacket {
            schema_version: TURN_RETRIEVAL_CONTROLLER_SCHEMA_VERSION.to_owned(),
            world_id: "stw_capsule".to_owned(),
            turn_id: "turn_0002".to_owned(),
            retrieval_cues: vec![TurnRetrievalCue {
                schema_version: TURN_RETRIEVAL_CUE_SCHEMA_VERSION.to_owned(),
                cue_id: "cue:turn_0002:hidden_gate".to_owned(),
                cue: "Gate access".to_owned(),
                reason: TurnRetrievalCueReason::CurrentGoalMatch,
                target_kinds: vec![TurnRetrievalTargetKind::WorldLore],
                visibility: TurnRetrievalVisibility::AdjudicationOnly,
                source_refs: vec!["hidden_state:gate".to_owned()],
            }],
            retrieval_policy: TurnRetrievalPolicy::default(),
            ..TurnRetrievalControllerPacket::default()
        };

        let selection = select_context_capsules(&ContextCapsuleSelectionInput {
            world_dir: temp.path(),
            world_id: "stw_capsule",
            turn_id: "turn_0002",
            player_input: "",
            retrieval_controller: &controller,
        })?;

        assert_eq!(
            selection.selected_capsules[0].reason,
            ContextCapsuleSelectionReason::ActiveProjectionFallback
        );
        Ok(())
    }

    #[test]
    fn repeated_fallback_capsule_is_rejected_without_current_effect() -> Result<()> {
        let temp = tempfile::tempdir()?;
        write_sample_registry(temp.path())?;
        append_prior_selection(temp.path(), "turn_0000")?;
        append_prior_selection(temp.path(), "turn_0001")?;

        let selection = select_context_capsules(&ContextCapsuleSelectionInput {
            world_dir: temp.path(),
            world_id: "stw_capsule",
            turn_id: "turn_0002",
            player_input: "",
            retrieval_controller: &TurnRetrievalControllerPacket::default(),
        })?;

        assert!(selection.selected_capsules.is_empty());
        assert!(selection.rejected_capsules.iter().any(|capsule| {
            capsule.reason == ContextCapsuleRejectReason::RepeatedWithoutEffect
        }));
        Ok(())
    }

    #[test]
    fn current_goal_match_overrides_recent_fallback_suppression() -> Result<()> {
        let temp = tempfile::tempdir()?;
        write_sample_registry(temp.path())?;
        append_prior_selection(temp.path(), "turn_0000")?;
        append_prior_selection(temp.path(), "turn_0001")?;
        let controller = TurnRetrievalControllerPacket {
            schema_version: TURN_RETRIEVAL_CONTROLLER_SCHEMA_VERSION.to_owned(),
            world_id: "stw_capsule".to_owned(),
            turn_id: "turn_0002".to_owned(),
            retrieval_cues: vec![TurnRetrievalCue {
                schema_version: TURN_RETRIEVAL_CUE_SCHEMA_VERSION.to_owned(),
                cue_id: "cue:turn_0002:gate".to_owned(),
                cue: "Gate access".to_owned(),
                reason: TurnRetrievalCueReason::CurrentGoalMatch,
                target_kinds: vec![TurnRetrievalTargetKind::WorldLore],
                visibility: TurnRetrievalVisibility::PlayerVisible,
                source_refs: vec!["plot_thread:gate".to_owned()],
            }],
            retrieval_policy: TurnRetrievalPolicy::default(),
            ..TurnRetrievalControllerPacket::default()
        };

        let selection = select_context_capsules(&ContextCapsuleSelectionInput {
            world_dir: temp.path(),
            world_id: "stw_capsule",
            turn_id: "turn_0002",
            player_input: "",
            retrieval_controller: &controller,
        })?;

        assert_eq!(selection.selected_capsules.len(), 1);
        assert_eq!(
            selection.selected_capsules[0].reason,
            ContextCapsuleSelectionReason::CurrentGoalMatch
        );
        assert!(!selection.rejected_capsules.iter().any(|capsule| {
            capsule.reason == ContextCapsuleRejectReason::RepeatedWithoutEffect
        }));
        Ok(())
    }

    #[test]
    fn direct_player_trigger_overrides_recent_fallback_repetition() -> Result<()> {
        let temp = tempfile::tempdir()?;
        write_sample_registry(temp.path())?;
        append_prior_selection(temp.path(), "turn_0000")?;
        append_prior_selection(temp.path(), "turn_0001")?;

        let selection = select_context_capsules(&ContextCapsuleSelectionInput {
            world_dir: temp.path(),
            world_id: "stw_capsule",
            turn_id: "turn_0002",
            player_input: "gate token을 살핀다",
            retrieval_controller: &TurnRetrievalControllerPacket::default(),
        })?;

        assert_eq!(selection.selected_capsules.len(), 1);
        assert_eq!(
            selection.selected_capsules[0].reason,
            ContextCapsuleSelectionReason::DirectPlayerInputTrigger
        );
        Ok(())
    }

    fn append_prior_selection(world_dir: &Path, turn_id: &str) -> Result<()> {
        append_jsonl(
            &world_dir.join(CONTEXT_CAPSULE_SELECTION_EVENTS_FILENAME),
            &ContextCapsuleSelectionEvent {
                schema_version: CONTEXT_CAPSULE_SELECTION_EVENT_SCHEMA_VERSION.to_owned(),
                world_id: "stw_capsule".to_owned(),
                turn_id: turn_id.to_owned(),
                event_id: format!("context_capsule_selection_event:{turn_id}"),
                selected_capsule_ids: vec!["world_lore:gate_custom".to_owned()],
                rejected_capsule_ids: Vec::new(),
                created_at: Utc::now().to_rfc3339(),
            },
        )
    }

    fn write_sample_registry(world_dir: &Path) -> Result<ContextCapsuleIndex> {
        let lore = WorldLorePacket {
            schema_version: WORLD_LORE_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_capsule".to_owned(),
            turn_id: "turn_0002".to_owned(),
            entries: vec![WorldLoreEntry {
                schema_version: WORLD_LORE_ENTRY_SCHEMA_VERSION.to_owned(),
                lore_id: "gate_custom".to_owned(),
                domain: WorldLoreDomain::Customs,
                name: "Gate custom".to_owned(),
                summary: "Gate officials require a local token.".to_owned(),
                visibility: "player_visible".to_owned(),
                confidence: "confirmed".to_owned(),
                authority: "test".to_owned(),
                source_refs: vec!["world_lore_update:turn_0001:00".to_owned()],
                mechanical_axis: vec!["social_permission".to_owned()],
            }],
            compiler_policy: WorldLorePolicy::default(),
        };
        let relationships = RelationshipGraphPacket {
            schema_version: RELATIONSHIP_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_capsule".to_owned(),
            turn_id: "turn_0002".to_owned(),
            active_edges: Vec::new(),
            compiler_policy: crate::relationship_graph::RelationshipGraphPolicy::default(),
        };
        let character_text = CharacterTextDesignPacket {
            schema_version: CHARACTER_TEXT_DESIGN_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_capsule".to_owned(),
            active_designs: Vec::new(),
            compiler_policy: CharacterTextDesignPolicy::default(),
        };
        rebuild_context_capsule_registry(&ContextCapsuleBuildInput {
            world_dir,
            world_id: "stw_capsule",
            turn_id: "turn_0002",
            active_world_lore: &lore,
            active_relationship_graph: &relationships,
            active_character_text_design: &character_text,
        })
    }
}
