use crate::character_text_design::CharacterTextDesignEventPlan;
use crate::plot_thread::PlotThreadEventPlan;
use crate::relationship_graph::RelationshipGraphEventPlan;
use crate::response_context::ContextVisibility;
use crate::scene_pressure::ScenePressureEventPlan;
use crate::store::{append_jsonl, read_json, write_json};
use crate::world_lore::WorldLoreUpdatePlan;
use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const CHANGE_LEDGER_SCHEMA_VERSION: &str = "singulari.change_ledger.v1";
pub const CHANGE_EVENT_SCHEMA_VERSION: &str = "singulari.change_event.v1";
pub const CHANGE_LEDGER_FILENAME: &str = "change_ledger.json";
pub const CHANGE_EVENTS_FILENAME: &str = "change_events.jsonl";

const ACTIVE_CHANGE_BUDGET: usize = 12;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChangeLedgerPacket {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub active_changes: Vec<ChangeEventRecord>,
    pub compiler_policy: ChangeLedgerPolicy,
}

impl Default for ChangeLedgerPacket {
    fn default() -> Self {
        Self {
            schema_version: CHANGE_LEDGER_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            turn_id: String::new(),
            active_changes: Vec::new(),
            compiler_policy: ChangeLedgerPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChangeEventPlan {
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub records: Vec<ChangeEventRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChangeEventRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub change_id: String,
    pub axis: ChangeAxis,
    pub target_id: String,
    pub before: String,
    pub after: String,
    #[serde(default)]
    pub cause_turns: Vec<String>,
    pub player_visible: bool,
    #[serde(default)]
    pub revival_triggers: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeAxis {
    Relationship,
    WorldLore,
    CharacterTextDesign,
    PlotThread,
    ScenePressure,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChangeLedgerPolicy {
    pub source: String,
    pub active_change_budget: usize,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for ChangeLedgerPolicy {
    fn default() -> Self {
        Self {
            source: "compiled_from_typed_turn_deltas_v1".to_owned(),
            active_change_budget: ACTIVE_CHANGE_BUDGET,
            use_rules: vec![
                "Change ledger records before/after pressure; it is not a raw event transcript."
                    .to_owned(),
                "Prompt revival should prefer player-visible changes over old raw facts when both explain the same current scene.".to_owned(),
                "Hidden or adjudication-only changes stay out of this player-visible packet.".to_owned(),
            ],
        }
    }
}

#[derive(Clone, Copy)]
pub struct ChangeEventPlanInput<'a> {
    pub world_id: &'a str,
    pub turn_id: &'a str,
    pub relationship_graph_events: &'a RelationshipGraphEventPlan,
    pub world_lore_updates: &'a WorldLoreUpdatePlan,
    pub character_text_design_events: &'a CharacterTextDesignEventPlan,
    pub plot_thread_events: &'a PlotThreadEventPlan,
    pub scene_pressure_events: &'a ScenePressureEventPlan,
}

/// Prepare change events from already validated typed delta plans.
///
/// # Errors
///
/// This currently has no additional fallible validation beyond returning the
/// same `Result` shape as neighboring preflight planners.
pub fn prepare_change_event_plan(input: ChangeEventPlanInput<'_>) -> Result<ChangeEventPlan> {
    let recorded_at = Utc::now().to_rfc3339();
    let mut records = Vec::new();
    collect_relationship_changes(&mut records, &recorded_at, &input);
    collect_world_lore_changes(&mut records, &recorded_at, &input);
    collect_character_text_design_changes(&mut records, &recorded_at, &input);
    collect_plot_thread_changes(&mut records, &recorded_at, &input);
    collect_scene_pressure_changes(&mut records, &recorded_at, &input);
    Ok(ChangeEventPlan {
        world_id: input.world_id.to_owned(),
        turn_id: input.turn_id.to_owned(),
        records,
    })
}

/// Append prevalidated change events.
///
/// # Errors
///
/// Returns an error when the durable event log cannot be written.
pub fn append_change_event_plan(world_dir: &Path, plan: &ChangeEventPlan) -> Result<()> {
    for record in &plan.records {
        append_jsonl(&world_dir.join(CHANGE_EVENTS_FILENAME), record)?;
    }
    Ok(())
}

/// Rebuild the player-visible change ledger from the durable change event log.
///
/// # Errors
///
/// Returns an error when change events cannot be loaded or the projection file
/// cannot be written.
pub fn rebuild_change_ledger(
    world_dir: &Path,
    base_packet: &ChangeLedgerPacket,
) -> Result<ChangeLedgerPacket> {
    let mut active_changes = load_change_event_records(world_dir)?
        .into_iter()
        .filter(|record| record.player_visible)
        .collect::<Vec<_>>();
    active_changes.reverse();
    active_changes.truncate(ACTIVE_CHANGE_BUDGET);
    active_changes.reverse();
    let packet = ChangeLedgerPacket {
        schema_version: CHANGE_LEDGER_SCHEMA_VERSION.to_owned(),
        world_id: base_packet.world_id.clone(),
        turn_id: base_packet.turn_id.clone(),
        active_changes,
        compiler_policy: ChangeLedgerPolicy::default(),
    };
    write_json(&world_dir.join(CHANGE_LEDGER_FILENAME), &packet)?;
    Ok(packet)
}

/// Load the materialized change ledger, or the supplied base packet for worlds
/// that have not materialized it yet.
///
/// # Errors
///
/// Returns an error when an existing materialized ledger cannot be parsed.
pub fn load_change_ledger_state(
    world_dir: &Path,
    base_packet: ChangeLedgerPacket,
) -> Result<ChangeLedgerPacket> {
    let path = world_dir.join(CHANGE_LEDGER_FILENAME);
    if path.is_file() {
        return read_json(&path);
    }
    Ok(base_packet)
}

fn load_change_event_records(world_dir: &Path) -> Result<Vec<ChangeEventRecord>> {
    let path = world_dir.join(CHANGE_EVENTS_FILENAME);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)?;
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str::<ChangeEventRecord>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn collect_relationship_changes(
    records: &mut Vec<ChangeEventRecord>,
    recorded_at: &str,
    input: &ChangeEventPlanInput<'_>,
) {
    for event in &input.relationship_graph_events.records {
        records.push(change_record(ChangeRecordSeed {
            input,
            recorded_at,
            index: records.len(),
            axis: ChangeAxis::Relationship,
            target_id: format!("{}->{}", event.source_entity_id, event.target_entity_id),
            before: "previous relationship stance".to_owned(),
            after: event.summary.clone(),
            player_visible: event.visibility == ContextVisibility::PlayerVisible,
            revival_triggers: vec![
                event.source_entity_id.clone(),
                event.target_entity_id.clone(),
                event.relation_kind.clone(),
            ],
            evidence_refs: event.evidence_refs.clone(),
        }));
    }
}

fn collect_world_lore_changes(
    records: &mut Vec<ChangeEventRecord>,
    recorded_at: &str,
    input: &ChangeEventPlanInput<'_>,
) {
    for update in &input.world_lore_updates.records {
        records.push(change_record(ChangeRecordSeed {
            input,
            recorded_at,
            index: records.len(),
            axis: ChangeAxis::WorldLore,
            target_id: format!("{}:{}", update.category, update.subject),
            before: "unrecorded or less specific lore".to_owned(),
            after: update.summary.clone(),
            player_visible: update.visibility == ContextVisibility::PlayerVisible,
            revival_triggers: vec![update.subject.clone(), update.category.clone()],
            evidence_refs: update.evidence_refs.clone(),
        }));
    }
}

fn collect_character_text_design_changes(
    records: &mut Vec<ChangeEventRecord>,
    recorded_at: &str,
    input: &ChangeEventPlanInput<'_>,
) {
    for event in &input.character_text_design_events.records {
        records.push(change_record(ChangeRecordSeed {
            input,
            recorded_at,
            index: records.len(),
            axis: ChangeAxis::CharacterTextDesign,
            target_id: event.character_id.clone(),
            before: "previous speech or gesture design".to_owned(),
            after: event.drift_note.clone(),
            player_visible: event.visibility == ContextVisibility::PlayerVisible,
            revival_triggers: vec![event.character_id.clone()],
            evidence_refs: event.evidence_refs.clone(),
        }));
    }
}

fn collect_plot_thread_changes(
    records: &mut Vec<ChangeEventRecord>,
    recorded_at: &str,
    input: &ChangeEventPlanInput<'_>,
) {
    for event in &input.plot_thread_events.records {
        records.push(change_record(ChangeRecordSeed {
            input,
            recorded_at,
            index: records.len(),
            axis: ChangeAxis::PlotThread,
            target_id: event.thread_id.clone(),
            before: "previous thread state".to_owned(),
            after: event.summary.clone(),
            player_visible: true,
            revival_triggers: vec![event.thread_id.clone(), format!("{:?}", event.status_after)],
            evidence_refs: event.evidence_refs.clone(),
        }));
    }
}

fn collect_scene_pressure_changes(
    records: &mut Vec<ChangeEventRecord>,
    recorded_at: &str,
    input: &ChangeEventPlanInput<'_>,
) {
    for event in &input.scene_pressure_events.records {
        records.push(change_record(ChangeRecordSeed {
            input,
            recorded_at,
            index: records.len(),
            axis: ChangeAxis::ScenePressure,
            target_id: event.pressure_id.clone(),
            before: "previous scene pressure".to_owned(),
            after: event.summary.clone(),
            player_visible: true,
            revival_triggers: vec![event.pressure_id.clone(), format!("{:?}", event.change)],
            evidence_refs: event.evidence_refs.clone(),
        }));
    }
}

struct ChangeRecordSeed<'a> {
    input: &'a ChangeEventPlanInput<'a>,
    recorded_at: &'a str,
    index: usize,
    axis: ChangeAxis,
    target_id: String,
    before: String,
    after: String,
    player_visible: bool,
    revival_triggers: Vec<String>,
    evidence_refs: Vec<String>,
}

fn change_record(seed: ChangeRecordSeed<'_>) -> ChangeEventRecord {
    ChangeEventRecord {
        schema_version: CHANGE_EVENT_SCHEMA_VERSION.to_owned(),
        world_id: seed.input.world_id.to_owned(),
        turn_id: seed.input.turn_id.to_owned(),
        change_id: format!(
            "change:{:?}:{}:{:02}",
            seed.axis, seed.input.turn_id, seed.index
        )
        .to_lowercase(),
        axis: seed.axis,
        target_id: seed.target_id,
        before: seed.before,
        after: seed.after,
        cause_turns: vec![seed.input.turn_id.to_owned()],
        player_visible: seed.player_visible,
        revival_triggers: seed.revival_triggers,
        evidence_refs: seed.evidence_refs,
        recorded_at: seed.recorded_at.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relationship_graph::{
        RELATIONSHIP_GRAPH_EVENT_SCHEMA_VERSION, RelationshipGraphEventRecord,
    };

    #[test]
    fn materializes_player_visible_relationship_change() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let plan = prepare_change_event_plan(ChangeEventPlanInput {
            world_id: "stw_change",
            turn_id: "turn_0003",
            relationship_graph_events: &RelationshipGraphEventPlan {
                world_id: "stw_change".to_owned(),
                turn_id: "turn_0003".to_owned(),
                records: vec![RelationshipGraphEventRecord {
                    schema_version: RELATIONSHIP_GRAPH_EVENT_SCHEMA_VERSION.to_owned(),
                    world_id: "stw_change".to_owned(),
                    turn_id: "turn_0003".to_owned(),
                    event_id: "relationship_graph_event:turn_0003:00".to_owned(),
                    source_entity_id: "char:protagonist".to_owned(),
                    target_entity_id: "char:porter".to_owned(),
                    relation_kind: "trust".to_owned(),
                    visibility: ContextVisibility::PlayerVisible,
                    summary: "문지기가 주인공을 경계하지만 말을 듣기 시작한다".to_owned(),
                    evidence_refs: vec!["canon_event:turn_0003".to_owned()],
                    recorded_at: "2026-04-29T00:00:00Z".to_owned(),
                }],
            },
            world_lore_updates: &WorldLoreUpdatePlan {
                world_id: "stw_change".to_owned(),
                turn_id: "turn_0003".to_owned(),
                records: Vec::new(),
            },
            character_text_design_events: &CharacterTextDesignEventPlan {
                world_id: "stw_change".to_owned(),
                turn_id: "turn_0003".to_owned(),
                records: Vec::new(),
            },
            plot_thread_events: &PlotThreadEventPlan {
                world_id: "stw_change".to_owned(),
                turn_id: "turn_0003".to_owned(),
                records: Vec::new(),
            },
            scene_pressure_events: &ScenePressureEventPlan {
                world_id: "stw_change".to_owned(),
                turn_id: "turn_0003".to_owned(),
                records: Vec::new(),
            },
        })?;

        append_change_event_plan(temp.path(), &plan)?;
        let packet = rebuild_change_ledger(
            temp.path(),
            &ChangeLedgerPacket {
                world_id: "stw_change".to_owned(),
                turn_id: "turn_0003".to_owned(),
                ..ChangeLedgerPacket::default()
            },
        )?;

        assert_eq!(packet.active_changes.len(), 1);
        assert_eq!(packet.active_changes[0].axis, ChangeAxis::Relationship);
        assert_eq!(
            packet.active_changes[0].target_id,
            "char:protagonist->char:porter"
        );
        Ok(())
    }
}
