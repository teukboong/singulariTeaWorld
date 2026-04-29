#![allow(clippy::missing_errors_doc)]

use crate::context_capsule::{ContextCapsuleIndex, ContextCapsuleKind};
use crate::plot_thread::PlotThreadPacket;
use crate::relationship_graph::RelationshipGraphPacket;
use crate::store::{read_json, write_json};
use crate::world_db::ChapterSummaryRecord;
use crate::world_process_clock::WorldProcessClockPacket;
use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

pub const AUTOBIOGRAPHICAL_INDEX_SCHEMA_VERSION: &str = "singulari.autobiographical_index.v1";
pub const AUTOBIOGRAPHICAL_PERIOD_SCHEMA_VERSION: &str = "singulari.autobiographical_period.v1";
pub const AUTOBIOGRAPHICAL_GENERAL_EVENT_SCHEMA_VERSION: &str =
    "singulari.autobiographical_general_event.v1";
pub const AUTOBIOGRAPHICAL_INDEX_FILENAME: &str = "autobiographical_index.json";

const PERIOD_BUDGET: usize = 8;
const GENERAL_EVENT_BUDGET: usize = 16;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutobiographicalIndexPacket {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub periods: Vec<AutobiographicalPeriod>,
    #[serde(default)]
    pub general_events: Vec<AutobiographicalGeneralEvent>,
    pub compiler_policy: AutobiographicalIndexPolicy,
    pub updated_at: String,
}

impl Default for AutobiographicalIndexPacket {
    fn default() -> Self {
        Self {
            schema_version: AUTOBIOGRAPHICAL_INDEX_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            turn_id: String::new(),
            periods: Vec::new(),
            general_events: Vec::new(),
            compiler_policy: AutobiographicalIndexPolicy::default(),
            updated_at: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutobiographicalPeriod {
    pub schema_version: String,
    pub period_id: String,
    pub label: String,
    pub turn_range: [String; 2],
    #[serde(default)]
    pub dominant_goals: Vec<String>,
    #[serde(default)]
    pub capsule_refs: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutobiographicalGeneralEvent {
    pub schema_version: String,
    pub event_cluster_id: String,
    pub pattern: String,
    pub event_kind: AutobiographicalGeneralEventKind,
    #[serde(default)]
    pub capsule_refs: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutobiographicalGeneralEventKind {
    PlotThread,
    RelationshipPattern,
    WorldProcess,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutobiographicalIndexPolicy {
    pub source: String,
    pub period_budget: usize,
    pub general_event_budget: usize,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for AutobiographicalIndexPolicy {
    fn default() -> Self {
        Self {
            source: "compiled_from_chapters_threads_relationships_processes_v1".to_owned(),
            period_budget: PERIOD_BUDGET,
            general_event_budget: GENERAL_EVENT_BUDGET,
            use_rules: vec![
                "Autobiographical index groups existing memory; it is not canon.".to_owned(),
                "Periods and general events may guide retrieval, but source evidence remains authoritative."
                    .to_owned(),
                "Do not infer protagonist personality, past life, or hidden motive from index labels."
                    .to_owned(),
            ],
        }
    }
}

pub struct AutobiographicalIndexInput<'a> {
    pub world_dir: &'a Path,
    pub world_id: &'a str,
    pub turn_id: &'a str,
    pub chapter_summaries: &'a [ChapterSummaryRecord],
    pub plot_threads: &'a PlotThreadPacket,
    pub relationship_graph: &'a RelationshipGraphPacket,
    pub world_process_clock: &'a WorldProcessClockPacket,
    pub context_capsule_index: &'a ContextCapsuleIndex,
}

pub fn rebuild_autobiographical_index(
    input: &AutobiographicalIndexInput<'_>,
) -> Result<AutobiographicalIndexPacket> {
    let mut packet = AutobiographicalIndexPacket {
        schema_version: AUTOBIOGRAPHICAL_INDEX_SCHEMA_VERSION.to_owned(),
        world_id: input.world_id.to_owned(),
        turn_id: input.turn_id.to_owned(),
        periods: compile_periods(input),
        general_events: compile_general_events(input),
        compiler_policy: AutobiographicalIndexPolicy::default(),
        updated_at: Utc::now().to_rfc3339(),
    };
    packet.periods.truncate(PERIOD_BUDGET);
    packet.general_events.truncate(GENERAL_EVENT_BUDGET);
    write_json(
        &input.world_dir.join(AUTOBIOGRAPHICAL_INDEX_FILENAME),
        &packet,
    )?;
    Ok(packet)
}

pub fn load_autobiographical_index_state(
    world_dir: &Path,
    fallback: AutobiographicalIndexPacket,
) -> Result<AutobiographicalIndexPacket> {
    let path = world_dir.join(AUTOBIOGRAPHICAL_INDEX_FILENAME);
    if path.is_file() {
        return read_json(&path);
    }
    Ok(fallback)
}

fn compile_periods(input: &AutobiographicalIndexInput<'_>) -> Vec<AutobiographicalPeriod> {
    let mut summaries = input.chapter_summaries.to_vec();
    summaries.sort_by_key(|summary| summary.chapter_index);
    summaries
        .into_iter()
        .rev()
        .map(|summary| {
            let dominant_goals = chapter_dominant_goals(&summary);
            let capsule_refs = capsule_refs_for_text(
                input.context_capsule_index,
                dominant_goals
                    .iter()
                    .chain(std::iter::once(&summary.summary))
                    .map(String::as_str),
            );
            AutobiographicalPeriod {
                schema_version: AUTOBIOGRAPHICAL_PERIOD_SCHEMA_VERSION.to_owned(),
                period_id: format!("period:{}", sanitize_ref(summary.summary_id.as_str())),
                label: summary.title,
                turn_range: [summary.source_turn_start, summary.source_turn_end],
                dominant_goals,
                capsule_refs,
                evidence_refs: vec![format!("chapter_summary:{}", summary.summary_id)],
            }
        })
        .collect()
}

fn compile_general_events(
    input: &AutobiographicalIndexInput<'_>,
) -> Vec<AutobiographicalGeneralEvent> {
    let mut events = Vec::new();
    for thread in &input.plot_threads.active_visible {
        let text = format!(
            "{} {} {}",
            thread.title, thread.summary, thread.current_question
        );
        events.push(AutobiographicalGeneralEvent {
            schema_version: AUTOBIOGRAPHICAL_GENERAL_EVENT_SCHEMA_VERSION.to_owned(),
            event_cluster_id: format!("general_event:plot:{}", sanitize_ref(&thread.thread_id)),
            pattern: format!("{}: {}", thread.title, thread.current_question),
            event_kind: AutobiographicalGeneralEventKind::PlotThread,
            capsule_refs: capsule_refs_for_text(input.context_capsule_index, [text.as_str()]),
            evidence_refs: thread.source_refs.clone(),
        });
    }
    for edge in &input.relationship_graph.active_edges {
        events.push(AutobiographicalGeneralEvent {
            schema_version: AUTOBIOGRAPHICAL_GENERAL_EVENT_SCHEMA_VERSION.to_owned(),
            event_cluster_id: format!("general_event:relationship:{}", sanitize_ref(&edge.edge_id)),
            pattern: edge.visible_summary.clone(),
            event_kind: AutobiographicalGeneralEventKind::RelationshipPattern,
            capsule_refs: capsule_refs_for_text(
                input.context_capsule_index,
                [
                    edge.source_entity_id.as_str(),
                    edge.target_entity_id.as_str(),
                    edge.stance.as_str(),
                    edge.visible_summary.as_str(),
                ],
            ),
            evidence_refs: edge.source_refs.clone(),
        });
    }
    for process in &input.world_process_clock.visible_processes {
        events.push(AutobiographicalGeneralEvent {
            schema_version: AUTOBIOGRAPHICAL_GENERAL_EVENT_SCHEMA_VERSION.to_owned(),
            event_cluster_id: format!(
                "general_event:process:{}",
                sanitize_ref(&process.process_id)
            ),
            pattern: process.next_tick_contract.clone(),
            event_kind: AutobiographicalGeneralEventKind::WorldProcess,
            capsule_refs: capsule_refs_for_text(
                input.context_capsule_index,
                [
                    process.summary.as_str(),
                    process.next_tick_contract.as_str(),
                ],
            ),
            evidence_refs: process.source_refs.clone(),
        });
    }
    events.sort_by_key(general_event_priority_key);
    events
}

fn chapter_dominant_goals(summary: &ChapterSummaryRecord) -> Vec<String> {
    let mut goals = summary
        .v2
        .open_ambiguities
        .iter()
        .chain(summary.v2.process_changes.iter())
        .chain(summary.v2.relationship_changes.iter())
        .chain(summary.v2.facts.iter().take(3))
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>();
    if goals.is_empty() && !summary.summary.trim().is_empty() {
        goals.push(summary.summary.clone());
    }
    goals.truncate(5);
    goals
}

fn capsule_refs_for_text<'a>(
    capsule_index: &ContextCapsuleIndex,
    parts: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let terms = trigger_terms(parts);
    let mut refs = BTreeSet::new();
    for entry in &capsule_index.entries {
        if matches!(entry.kind, ContextCapsuleKind::CharacterTextDesign)
            && !terms.iter().any(|term| entry.triggers.contains(term))
        {
            continue;
        }
        if entry.triggers.iter().any(|trigger| {
            terms
                .iter()
                .any(|term| trigger.contains(term) || term.contains(trigger))
        }) {
            refs.insert(entry.capsule_id.clone());
        }
    }
    refs.into_iter().collect()
}

fn trigger_terms<'a>(parts: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    let mut terms = BTreeSet::new();
    for part in parts {
        for raw in part
            .split(|ch: char| !ch.is_alphanumeric() && ch != ':' && ch != '_')
            .map(str::trim)
            .filter(|value| value.chars().count() >= 3)
        {
            terms.insert(raw.to_lowercase());
        }
    }
    terms
}

fn general_event_priority_key(event: &AutobiographicalGeneralEvent) -> (u8, String) {
    let priority = match event.event_kind {
        AutobiographicalGeneralEventKind::PlotThread => 0,
        AutobiographicalGeneralEventKind::RelationshipPattern => 1,
        AutobiographicalGeneralEventKind::WorldProcess => 2,
    };
    (priority, event.event_cluster_id.clone())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_capsule::{
        CONTEXT_CAPSULE_INDEX_ENTRY_SCHEMA_VERSION, CONTEXT_CAPSULE_INDEX_SCHEMA_VERSION,
        ContextCapsuleIndexEntry, ContextCapsulePolicy, ContextCapsuleVisibility,
    };
    use crate::plot_thread::{
        PLOT_THREAD_PACKET_SCHEMA_VERSION, PLOT_THREAD_SCHEMA_VERSION, PlotThread, PlotThreadKind,
        PlotThreadPolicy, PlotThreadStatus, PlotThreadUrgency,
    };
    use crate::relationship_graph::{
        RELATIONSHIP_EDGE_SCHEMA_VERSION, RELATIONSHIP_GRAPH_PACKET_SCHEMA_VERSION,
        RelationshipEdge, RelationshipGraphPolicy,
    };
    use crate::world_db::ChapterSummaryV2;
    use crate::world_process_clock::{
        WORLD_PROCESS_CLOCK_PACKET_SCHEMA_VERSION, WORLD_PROCESS_SCHEMA_VERSION, WorldProcess,
        WorldProcessClockPolicy, WorldProcessTempo, WorldProcessVisibility,
    };

    #[test]
    fn builds_periods_and_general_events_from_existing_memory_layers() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let index = rebuild_autobiographical_index(&AutobiographicalIndexInput {
            world_dir: temp.path(),
            world_id: "stw_auto_index",
            turn_id: "turn_0005",
            chapter_summaries: &[sample_chapter()],
            plot_threads: &sample_threads(),
            relationship_graph: &sample_relationships(),
            world_process_clock: &sample_processes(),
            context_capsule_index: &sample_capsules(),
        })?;

        assert_eq!(index.periods.len(), 1);
        assert!(
            index.periods[0]
                .capsule_refs
                .contains(&"world_lore:gate_tax".to_owned())
        );
        assert!(index.general_events.iter().any(|event| {
            event.event_kind == AutobiographicalGeneralEventKind::PlotThread
                && event
                    .capsule_refs
                    .contains(&"world_lore:gate_tax".to_owned())
        }));
        assert!(temp.path().join(AUTOBIOGRAPHICAL_INDEX_FILENAME).is_file());
        Ok(())
    }

    fn sample_chapter() -> ChapterSummaryRecord {
        ChapterSummaryRecord {
            summary_id: "chapter_0001".to_owned(),
            chapter_index: 1,
            title: "Chapter 1: gate".to_owned(),
            summary: "gate tax requires a stamped token".to_owned(),
            v2: ChapterSummaryV2 {
                summary_id: "chapter_0001".to_owned(),
                source_turn_start: "turn_0000".to_owned(),
                source_turn_end: "turn_0004".to_owned(),
                facts: vec!["gate tax requires a stamped token".to_owned()],
                ..ChapterSummaryV2::default()
            },
            source_turn_start: "turn_0000".to_owned(),
            source_turn_end: "turn_0004".to_owned(),
            created_at: "2026-04-29T00:00:00Z".to_owned(),
        }
    }

    fn sample_capsules() -> ContextCapsuleIndex {
        ContextCapsuleIndex {
            schema_version: CONTEXT_CAPSULE_INDEX_SCHEMA_VERSION.to_owned(),
            world_id: "stw_auto_index".to_owned(),
            turn_id: "turn_0005".to_owned(),
            entries: vec![ContextCapsuleIndexEntry {
                schema_version: CONTEXT_CAPSULE_INDEX_ENTRY_SCHEMA_VERSION.to_owned(),
                world_id: "stw_auto_index".to_owned(),
                capsule_id: "world_lore:gate_tax".to_owned(),
                kind: ContextCapsuleKind::WorldLore,
                visibility: ContextCapsuleVisibility::PlayerVisible,
                summary: "gate tax requires a stamped token".to_owned(),
                triggers: vec!["gate".to_owned(), "tax".to_owned(), "token".to_owned()],
                evidence_refs: Vec::new(),
                content_ref: "context_capsules/world_lore/gate_tax.json".to_owned(),
                token_estimate: 12,
                last_used_turn: None,
                recent_use_count: 0,
                updated_at: "2026-04-29T00:00:00Z".to_owned(),
            }],
            compiler_policy: ContextCapsulePolicy::default(),
        }
    }

    fn sample_threads() -> PlotThreadPacket {
        PlotThreadPacket {
            schema_version: PLOT_THREAD_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_auto_index".to_owned(),
            turn_id: "turn_0005".to_owned(),
            active_visible: vec![PlotThread {
                schema_version: PLOT_THREAD_SCHEMA_VERSION.to_owned(),
                thread_id: "thread:gate_tax".to_owned(),
                title: "Gate tax".to_owned(),
                thread_kind: PlotThreadKind::Access,
                status: PlotThreadStatus::Active,
                urgency: PlotThreadUrgency::Immediate,
                summary: "The gate tax blocks passage.".to_owned(),
                current_question: "How can the protagonist secure a token?".to_owned(),
                source_refs: vec!["canon_event:turn_0002".to_owned()],
                next_scene_hooks: Vec::new(),
            }],
            compiler_policy: PlotThreadPolicy::default(),
        }
    }

    fn sample_relationships() -> RelationshipGraphPacket {
        RelationshipGraphPacket {
            schema_version: RELATIONSHIP_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_auto_index".to_owned(),
            turn_id: "turn_0005".to_owned(),
            active_edges: vec![RelationshipEdge {
                schema_version: RELATIONSHIP_EDGE_SCHEMA_VERSION.to_owned(),
                edge_id: "guard->protagonist:suspicious".to_owned(),
                source_entity_id: "char:guard".to_owned(),
                target_entity_id: "char:protagonist".to_owned(),
                stance: "suspicious".to_owned(),
                visibility: "player_visible".to_owned(),
                visible_summary: "The guard doubts the protagonist near the gate.".to_owned(),
                source_refs: vec!["relationship_graph_event:turn_0002:00".to_owned()],
                voice_effects: Vec::new(),
            }],
            compiler_policy: RelationshipGraphPolicy::default(),
        }
    }

    fn sample_processes() -> WorldProcessClockPacket {
        WorldProcessClockPacket {
            schema_version: WORLD_PROCESS_CLOCK_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_auto_index".to_owned(),
            turn_id: "turn_0005".to_owned(),
            visible_processes: vec![WorldProcess {
                schema_version: WORLD_PROCESS_SCHEMA_VERSION.to_owned(),
                process_id: "process:gate_closing".to_owned(),
                visibility: WorldProcessVisibility::PlayerVisible,
                tempo: WorldProcessTempo::Soon,
                summary: "The gate will close soon.".to_owned(),
                next_tick_contract: "Gate access narrows unless a token is secured.".to_owned(),
                source_refs: vec!["scene_pressure:gate_time".to_owned()],
            }],
            adjudication_only_processes: Vec::new(),
            compiler_policy: WorldProcessClockPolicy::default(),
        }
    }
}
