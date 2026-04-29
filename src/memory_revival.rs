use crate::character_text_design::{CharacterTextDesign, CharacterTextDesignPacket};
use crate::extra_memory::{ExtraMemoryPacket, ExtraTrace, RememberedExtra};
use crate::relationship_graph::{RelationshipEdge, RelationshipGraphPacket};
use crate::store::append_jsonl;
use crate::world_lore::{WorldLoreEntry, WorldLorePacket};
use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub const MEMORY_REVIVAL_ITEM_SCHEMA_VERSION: &str = "singulari.memory_revival_item.v1";
pub const MEMORY_REVIVAL_EVENT_SCHEMA_VERSION: &str = "singulari.memory_revival_event.v1";
pub const MEMORY_REVIVAL_EVENTS_FILENAME: &str = "memory_revival_events.jsonl";

const WORLD_LORE_BUDGET: usize = 8;
const RELATIONSHIP_EDGE_BUDGET: usize = 8;
const CHARACTER_TEXT_DESIGN_BUDGET: usize = 8;
const REMEMBERED_EXTRA_BUDGET: usize = 7;
const EXTRA_TRACE_BUDGET: usize = 5;
const RECENT_EVENT_WINDOW: usize = 6;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryRevivalSelection {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub backend: String,
    #[serde(default)]
    pub selected_items: Vec<MemoryRevivalItem>,
    pub event: MemoryRevivalEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryRevivalItem {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub source_kind: MemoryRevivalSourceKind,
    pub source_id: String,
    pub visibility: String,
    pub reason: MemoryRevivalReason,
    pub score: f32,
    pub payload: Value,
    #[serde(default)]
    pub evidence_refs: Vec<MemoryRevivalEvidenceRef>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum MemoryRevivalSourceKind {
    WorldLoreEntry,
    RelationshipEdge,
    CharacterTextDesign,
    RememberedExtra,
    ExtraTrace,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum MemoryRevivalReason {
    PlayerInputMatch,
    CurrentLocationMatch,
    RecentContinuity,
    ActiveDesignContract,
    PriorContactHook,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryRevivalEvidenceRef {
    pub source: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryRevivalEvent {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub event_id: String,
    pub backend: String,
    pub selected_counts: BTreeMap<MemoryRevivalSourceKind, usize>,
    pub rejected_counts: BTreeMap<MemoryRevivalRejectReason, usize>,
    #[serde(default)]
    pub selected_source_ids: Vec<String>,
    #[serde(default)]
    pub top_reasons: Vec<MemoryRevivalReason>,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum MemoryRevivalRejectReason {
    StaleRepeated,
    BudgetExceeded,
    ZeroScore,
}

#[derive(Debug, Clone)]
pub struct MemoryRevivalCompileInput<'a> {
    pub world_dir: &'a Path,
    pub world_id: &'a str,
    pub turn_id: &'a str,
    pub backend: &'a str,
    pub player_input: &'a str,
    pub current_location_id: &'a str,
    pub active_world_lore: &'a WorldLorePacket,
    pub active_relationship_graph: &'a RelationshipGraphPacket,
    pub active_character_text_design: &'a CharacterTextDesignPacket,
    pub extra_memory: &'a ExtraMemoryPacket,
}

/// Compile and audit selected long-memory items for a turn.
///
/// # Errors
///
/// Returns an error when the prior revival audit log cannot be parsed or the
/// new audit event cannot be written.
pub fn compile_memory_revival_selection(
    input: &MemoryRevivalCompileInput<'_>,
) -> Result<MemoryRevivalSelection> {
    let prior_events = load_memory_revival_events(input.world_dir)?;
    let recent_usage = recent_usage_counts(&prior_events);
    let mut rejected_counts = BTreeMap::new();
    let mut candidates = Vec::new();

    collect_world_lore_candidates(input, &recent_usage, &mut candidates, &mut rejected_counts);
    collect_relationship_candidates(input, &recent_usage, &mut candidates, &mut rejected_counts);
    collect_character_design_candidates(
        input,
        &recent_usage,
        &mut candidates,
        &mut rejected_counts,
    );
    collect_extra_memory_candidates(input, &recent_usage, &mut candidates, &mut rejected_counts);

    let selected_items = select_budgeted_items(candidates, &mut rejected_counts);
    let event = memory_revival_event(input, &selected_items, rejected_counts);
    append_jsonl(
        &input.world_dir.join(MEMORY_REVIVAL_EVENTS_FILENAME),
        &event,
    )?;

    Ok(MemoryRevivalSelection {
        schema_version: "singulari.memory_revival_selection.v1".to_owned(),
        world_id: input.world_id.to_owned(),
        turn_id: input.turn_id.to_owned(),
        backend: input.backend.to_owned(),
        selected_items,
        event,
    })
}

/// Load memory revival audit events.
///
/// # Errors
///
/// Returns an error when the event log exists but contains malformed JSON.
pub fn load_memory_revival_events(world_dir: &Path) -> Result<Vec<MemoryRevivalEvent>> {
    let path = world_dir.join(MEMORY_REVIVAL_EVENTS_FILENAME);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(path.as_path())
        .with_context(|| format!("failed to read {}", path.display()))?;
    raw.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str::<MemoryRevivalEvent>(line)
                .with_context(|| format!("failed to parse {} line {}", path.display(), index + 1))
        })
        .collect()
}

fn collect_world_lore_candidates(
    input: &MemoryRevivalCompileInput<'_>,
    recent_usage: &BTreeMap<String, usize>,
    candidates: &mut Vec<MemoryRevivalItem>,
    rejected_counts: &mut BTreeMap<MemoryRevivalRejectReason, usize>,
) {
    for entry in &input.active_world_lore.entries {
        push_candidate(
            input,
            candidates,
            rejected_counts,
            recent_usage,
            world_lore_item(input, entry),
        );
    }
}

fn collect_relationship_candidates(
    input: &MemoryRevivalCompileInput<'_>,
    recent_usage: &BTreeMap<String, usize>,
    candidates: &mut Vec<MemoryRevivalItem>,
    rejected_counts: &mut BTreeMap<MemoryRevivalRejectReason, usize>,
) {
    for edge in &input.active_relationship_graph.active_edges {
        push_candidate(
            input,
            candidates,
            rejected_counts,
            recent_usage,
            relationship_item(input, edge),
        );
    }
}

fn collect_character_design_candidates(
    input: &MemoryRevivalCompileInput<'_>,
    recent_usage: &BTreeMap<String, usize>,
    candidates: &mut Vec<MemoryRevivalItem>,
    rejected_counts: &mut BTreeMap<MemoryRevivalRejectReason, usize>,
) {
    for design in &input.active_character_text_design.active_designs {
        push_candidate(
            input,
            candidates,
            rejected_counts,
            recent_usage,
            character_design_item(input, design),
        );
    }
}

fn collect_extra_memory_candidates(
    input: &MemoryRevivalCompileInput<'_>,
    recent_usage: &BTreeMap<String, usize>,
    candidates: &mut Vec<MemoryRevivalItem>,
    rejected_counts: &mut BTreeMap<MemoryRevivalRejectReason, usize>,
) {
    for extra in &input.extra_memory.remembered_extras {
        push_candidate(
            input,
            candidates,
            rejected_counts,
            recent_usage,
            remembered_extra_item(input, extra),
        );
    }
    for trace in &input.extra_memory.recent_extra_traces {
        push_candidate(
            input,
            candidates,
            rejected_counts,
            recent_usage,
            extra_trace_item(input, trace),
        );
    }
}

fn push_candidate(
    input: &MemoryRevivalCompileInput<'_>,
    candidates: &mut Vec<MemoryRevivalItem>,
    rejected_counts: &mut BTreeMap<MemoryRevivalRejectReason, usize>,
    recent_usage: &BTreeMap<String, usize>,
    mut item: MemoryRevivalItem,
) {
    let direct_match = text_matches(item.payload.to_string().as_str(), input.player_input)
        || text_matches(item.source_id.as_str(), input.player_input);
    if let Some(count) = recent_usage.get(item.source_id.as_str())
        && *count >= 2
        && !direct_match
    {
        increment_reject(rejected_counts, MemoryRevivalRejectReason::StaleRepeated);
        return;
    }
    if !direct_match && item.score <= 0.0 {
        increment_reject(rejected_counts, MemoryRevivalRejectReason::ZeroScore);
        return;
    }
    if direct_match {
        item.reason = MemoryRevivalReason::PlayerInputMatch;
        item.score = (item.score + 1.0).min(1.0);
    }
    candidates.push(item);
}

fn select_budgeted_items(
    mut candidates: Vec<MemoryRevivalItem>,
    rejected_counts: &mut BTreeMap<MemoryRevivalRejectReason, usize>,
) -> Vec<MemoryRevivalItem> {
    candidates.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.source_id.cmp(&right.source_id))
    });
    let mut selected = Vec::new();
    let mut selected_counts = BTreeMap::new();
    for item in candidates {
        let budget = budget_for_source(item.source_kind);
        let count = selected_counts.entry(item.source_kind).or_insert(0);
        if *count >= budget {
            increment_reject(rejected_counts, MemoryRevivalRejectReason::BudgetExceeded);
            continue;
        }
        *count += 1;
        selected.push(item);
    }
    selected
}

fn memory_revival_event(
    input: &MemoryRevivalCompileInput<'_>,
    selected_items: &[MemoryRevivalItem],
    rejected_counts: BTreeMap<MemoryRevivalRejectReason, usize>,
) -> MemoryRevivalEvent {
    let mut selected_counts = BTreeMap::new();
    let mut reason_counts = BTreeMap::new();
    for item in selected_items {
        *selected_counts.entry(item.source_kind).or_insert(0) += 1;
        *reason_counts.entry(item.reason).or_insert(0usize) += 1;
    }
    let mut top_reasons = reason_counts.into_iter().collect::<Vec<_>>();
    top_reasons.sort_by(|(left_reason, left_count), (right_reason, right_count)| {
        right_count
            .cmp(left_count)
            .then_with(|| left_reason.cmp(right_reason))
    });
    MemoryRevivalEvent {
        schema_version: MEMORY_REVIVAL_EVENT_SCHEMA_VERSION.to_owned(),
        world_id: input.world_id.to_owned(),
        turn_id: input.turn_id.to_owned(),
        event_id: format!("memory_revival_event:{}", input.turn_id),
        backend: input.backend.to_owned(),
        selected_counts,
        rejected_counts,
        top_reasons: top_reasons
            .into_iter()
            .map(|(reason, _)| reason)
            .take(5)
            .collect(),
        selected_source_ids: selected_items
            .iter()
            .map(|item| item.source_id.clone())
            .collect(),
        created_at: Utc::now().to_rfc3339(),
    }
}

fn world_lore_item(
    input: &MemoryRevivalCompileInput<'_>,
    entry: &WorldLoreEntry,
) -> MemoryRevivalItem {
    let score = if text_matches(entry.summary.as_str(), input.player_input) {
        1.0
    } else {
        0.72
    };
    MemoryRevivalItem {
        schema_version: MEMORY_REVIVAL_ITEM_SCHEMA_VERSION.to_owned(),
        world_id: input.world_id.to_owned(),
        turn_id: input.turn_id.to_owned(),
        item_id: format!(
            "revival:{}:{}",
            input.turn_id,
            stable_item_id(entry.lore_id.as_str())
        ),
        source_kind: MemoryRevivalSourceKind::WorldLoreEntry,
        source_id: entry.lore_id.clone(),
        visibility: entry.visibility.clone(),
        reason: MemoryRevivalReason::RecentContinuity,
        score,
        payload: serde_json::json!(entry),
        evidence_refs: entry
            .source_refs
            .iter()
            .map(|id| MemoryRevivalEvidenceRef {
                source: "world_lore.json".to_owned(),
                id: id.clone(),
            })
            .collect(),
    }
}

fn relationship_item(
    input: &MemoryRevivalCompileInput<'_>,
    edge: &RelationshipEdge,
) -> MemoryRevivalItem {
    let score = if text_matches(edge.visible_summary.as_str(), input.player_input) {
        1.0
    } else {
        0.76
    };
    MemoryRevivalItem {
        schema_version: MEMORY_REVIVAL_ITEM_SCHEMA_VERSION.to_owned(),
        world_id: input.world_id.to_owned(),
        turn_id: input.turn_id.to_owned(),
        item_id: format!(
            "revival:{}:{}",
            input.turn_id,
            stable_item_id(edge.edge_id.as_str())
        ),
        source_kind: MemoryRevivalSourceKind::RelationshipEdge,
        source_id: edge.edge_id.clone(),
        visibility: edge.visibility.clone(),
        reason: MemoryRevivalReason::RecentContinuity,
        score,
        payload: serde_json::json!(edge),
        evidence_refs: edge
            .source_refs
            .iter()
            .map(|id| MemoryRevivalEvidenceRef {
                source: "relationship_graph.json".to_owned(),
                id: id.clone(),
            })
            .collect(),
    }
}

fn character_design_item(
    input: &MemoryRevivalCompileInput<'_>,
    design: &CharacterTextDesign,
) -> MemoryRevivalItem {
    MemoryRevivalItem {
        schema_version: MEMORY_REVIVAL_ITEM_SCHEMA_VERSION.to_owned(),
        world_id: input.world_id.to_owned(),
        turn_id: input.turn_id.to_owned(),
        item_id: format!(
            "revival:{}:{}",
            input.turn_id,
            stable_item_id(design.entity_id.as_str())
        ),
        source_kind: MemoryRevivalSourceKind::CharacterTextDesign,
        source_id: design.entity_id.clone(),
        visibility: design.visibility.clone(),
        reason: MemoryRevivalReason::ActiveDesignContract,
        score: if text_matches(design.visible_name.as_str(), input.player_input) {
            1.0
        } else {
            0.68
        },
        payload: serde_json::json!(design),
        evidence_refs: design
            .source_refs
            .iter()
            .map(|id| MemoryRevivalEvidenceRef {
                source: "character_text_design.json".to_owned(),
                id: id.clone(),
            })
            .collect(),
    }
}

fn remembered_extra_item(
    input: &MemoryRevivalCompileInput<'_>,
    extra: &RememberedExtra,
) -> MemoryRevivalItem {
    let location_match = extra.home_location_id == input.current_location_id;
    MemoryRevivalItem {
        schema_version: MEMORY_REVIVAL_ITEM_SCHEMA_VERSION.to_owned(),
        world_id: input.world_id.to_owned(),
        turn_id: input.turn_id.to_owned(),
        item_id: format!(
            "revival:{}:{}",
            input.turn_id,
            stable_item_id(extra.extra_id.as_str())
        ),
        source_kind: MemoryRevivalSourceKind::RememberedExtra,
        source_id: extra.extra_id.clone(),
        visibility: extra.visibility.clone(),
        reason: if location_match {
            MemoryRevivalReason::CurrentLocationMatch
        } else {
            MemoryRevivalReason::PriorContactHook
        },
        score: if location_match { 0.9 } else { 0.55 },
        payload: serde_json::json!(extra),
        evidence_refs: vec![MemoryRevivalEvidenceRef {
            source: "remembered_extras.json".to_owned(),
            id: extra.extra_id.clone(),
        }],
    }
}

fn extra_trace_item(
    input: &MemoryRevivalCompileInput<'_>,
    trace: &ExtraTrace,
) -> MemoryRevivalItem {
    let location_match = trace.location_id == input.current_location_id;
    MemoryRevivalItem {
        schema_version: MEMORY_REVIVAL_ITEM_SCHEMA_VERSION.to_owned(),
        world_id: input.world_id.to_owned(),
        turn_id: input.turn_id.to_owned(),
        item_id: format!(
            "revival:{}:{}",
            input.turn_id,
            stable_item_id(trace.trace_id.as_str())
        ),
        source_kind: MemoryRevivalSourceKind::ExtraTrace,
        source_id: trace.trace_id.clone(),
        visibility: "player_visible".to_owned(),
        reason: if location_match {
            MemoryRevivalReason::CurrentLocationMatch
        } else {
            MemoryRevivalReason::RecentContinuity
        },
        score: if location_match { 0.74 } else { 0.42 },
        payload: serde_json::json!(trace),
        evidence_refs: vec![MemoryRevivalEvidenceRef {
            source: "extra_traces.jsonl".to_owned(),
            id: trace.trace_id.clone(),
        }],
    }
}

fn recent_usage_counts(events: &[MemoryRevivalEvent]) -> BTreeMap<String, usize> {
    events
        .iter()
        .rev()
        .take(RECENT_EVENT_WINDOW)
        .flat_map(|event| event.selected_source_ids.iter().cloned())
        .fold(BTreeMap::new(), |mut counts, source_id| {
            *counts.entry(source_id).or_insert(0) += 1;
            counts
        })
}

fn budget_for_source(source_kind: MemoryRevivalSourceKind) -> usize {
    match source_kind {
        MemoryRevivalSourceKind::WorldLoreEntry => WORLD_LORE_BUDGET,
        MemoryRevivalSourceKind::RelationshipEdge => RELATIONSHIP_EDGE_BUDGET,
        MemoryRevivalSourceKind::CharacterTextDesign => CHARACTER_TEXT_DESIGN_BUDGET,
        MemoryRevivalSourceKind::RememberedExtra => REMEMBERED_EXTRA_BUDGET,
        MemoryRevivalSourceKind::ExtraTrace => EXTRA_TRACE_BUDGET,
    }
}

fn increment_reject(
    rejected_counts: &mut BTreeMap<MemoryRevivalRejectReason, usize>,
    reason: MemoryRevivalRejectReason,
) {
    *rejected_counts.entry(reason).or_insert(0) += 1;
}

fn text_matches(text: &str, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return false;
    }
    let text = text.to_lowercase();
    query
        .split_whitespace()
        .filter(|token| token.chars().count() >= 2)
        .any(|token| text.contains(token))
}

fn stable_item_id(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_revival_items_with_counts_and_reasons() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let lore = WorldLorePacket {
            world_id: "stw_memory".to_owned(),
            turn_id: "turn_0002".to_owned(),
            entries: vec![WorldLoreEntry {
                schema_version: crate::world_lore::WORLD_LORE_ENTRY_SCHEMA_VERSION.to_owned(),
                lore_id: "lore:gate_tax".to_owned(),
                domain: crate::world_lore::WorldLoreDomain::Customs,
                name: "gate tax".to_owned(),
                summary: "gate tax requires a stamped token".to_owned(),
                visibility: "player_visible".to_owned(),
                confidence: "confirmed".to_owned(),
                authority: "world_lore_updates".to_owned(),
                source_refs: vec!["world_lore_update:turn_0001:00".to_owned()],
                mechanical_axis: vec!["social_permission".to_owned()],
            }],
            ..WorldLorePacket::default()
        };
        let relationships = RelationshipGraphPacket::default();
        let designs = CharacterTextDesignPacket::default();
        let extra_memory = ExtraMemoryPacket::default();

        let selection = compile_memory_revival_selection(&MemoryRevivalCompileInput {
            world_dir: temp.path(),
            world_id: "stw_memory",
            turn_id: "turn_0002",
            backend: "webgpt_text",
            player_input: "gate tax를 확인한다",
            current_location_id: "place:gate",
            active_world_lore: &lore,
            active_relationship_graph: &relationships,
            active_character_text_design: &designs,
            extra_memory: &extra_memory,
        })?;

        assert_eq!(selection.selected_items.len(), 1);
        assert_eq!(
            selection.selected_items[0].reason,
            MemoryRevivalReason::PlayerInputMatch
        );
        assert!(temp.path().join(MEMORY_REVIVAL_EVENTS_FILENAME).is_file());
        Ok(())
    }

    #[test]
    fn suppresses_stale_repeated_items_without_direct_input_match() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let lore = WorldLorePacket {
            world_id: "stw_memory".to_owned(),
            turn_id: "turn_0002".to_owned(),
            entries: vec![WorldLoreEntry {
                schema_version: crate::world_lore::WORLD_LORE_ENTRY_SCHEMA_VERSION.to_owned(),
                lore_id: "lore:gate_tax".to_owned(),
                domain: crate::world_lore::WorldLoreDomain::Customs,
                name: "gate tax".to_owned(),
                summary: "gate tax requires a stamped token".to_owned(),
                visibility: "player_visible".to_owned(),
                confidence: "confirmed".to_owned(),
                authority: "world_lore_updates".to_owned(),
                source_refs: vec!["world_lore_update:turn_0001:00".to_owned()],
                mechanical_axis: vec!["social_permission".to_owned()],
            }],
            ..WorldLorePacket::default()
        };
        let relationships = RelationshipGraphPacket::default();
        let designs = CharacterTextDesignPacket::default();
        let extra_memory = ExtraMemoryPacket::default();

        for turn_id in ["turn_0002", "turn_0003"] {
            let selection = compile_memory_revival_selection(&MemoryRevivalCompileInput {
                world_dir: temp.path(),
                world_id: "stw_memory",
                turn_id,
                backend: "webgpt_text",
                player_input: "주변을 살핀다",
                current_location_id: "place:gate",
                active_world_lore: &lore,
                active_relationship_graph: &relationships,
                active_character_text_design: &designs,
                extra_memory: &extra_memory,
            })?;
            assert_eq!(selection.selected_items.len(), 1);
        }

        let selection = compile_memory_revival_selection(&MemoryRevivalCompileInput {
            world_dir: temp.path(),
            world_id: "stw_memory",
            turn_id: "turn_0004",
            backend: "webgpt_text",
            player_input: "주변을 살핀다",
            current_location_id: "place:gate",
            active_world_lore: &lore,
            active_relationship_graph: &relationships,
            active_character_text_design: &designs,
            extra_memory: &extra_memory,
        })?;

        assert!(selection.selected_items.is_empty());
        assert_eq!(
            selection
                .event
                .rejected_counts
                .get(&MemoryRevivalRejectReason::StaleRepeated)
                .copied(),
            Some(1)
        );
        Ok(())
    }
}
