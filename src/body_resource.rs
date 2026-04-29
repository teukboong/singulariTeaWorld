// Projection APIs return anyhow::Result with per-call path/context details; the
// Rustdoc error lists would duplicate those local error messages.
#![allow(clippy::missing_errors_doc)]

use crate::models::TurnSnapshot;
use crate::store::{append_jsonl, read_json, write_json};
use anyhow::{Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const BODY_RESOURCE_PACKET_SCHEMA_VERSION: &str = "singulari.body_resource_packet.v1";
pub const BODY_CONSTRAINT_SCHEMA_VERSION: &str = "singulari.body_constraint.v1";
pub const RESOURCE_ITEM_SCHEMA_VERSION: &str = "singulari.resource_item.v1";
pub const BODY_RESOURCE_EVENT_SCHEMA_VERSION: &str = "singulari.body_resource_event.v1";
pub const BODY_RESOURCE_EVENTS_FILENAME: &str = "body_resource_events.jsonl";
pub const BODY_RESOURCE_STATE_FILENAME: &str = "body_resource_state.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BodyResourcePacket {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub body_constraints: Vec<BodyConstraint>,
    #[serde(default)]
    pub resources: Vec<ResourceItem>,
    pub compiler_policy: BodyResourcePolicy,
}

impl Default for BodyResourcePacket {
    fn default() -> Self {
        Self {
            schema_version: BODY_RESOURCE_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            turn_id: String::new(),
            body_constraints: Vec::new(),
            resources: Vec::new(),
            compiler_policy: BodyResourcePolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BodyConstraint {
    pub schema_version: String,
    pub constraint_id: String,
    pub visibility: BodyResourceVisibility,
    pub summary: String,
    pub severity: u8,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default)]
    pub scene_pressure_kinds: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceItem {
    pub schema_version: String,
    pub resource_id: String,
    pub visibility: BodyResourceVisibility,
    pub summary: String,
    pub resource_kind: ResourceKind,
    #[serde(default)]
    pub source_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BodyResourcePolicy {
    pub source: String,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for BodyResourcePolicy {
    fn default() -> Self {
        Self {
            source: "compiled_from_protagonist_state_v0".to_owned(),
            use_rules: vec![
                "Body/resource state constrains choices and prose only when the current action touches it.".to_owned(),
                "Do not invent resources that are not in protagonist_state.inventory.".to_owned(),
                "Render labels and visible consequences, not raw stat math.".to_owned(),
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BodyResourceVisibility {
    PlayerVisible,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Money,
    Food,
    Water,
    Tool,
    Weapon,
    Document,
    Clothing,
    TradeGood,
    Medicine,
    Shelter,
    Transport,
    SocialCover,
    InformationToken,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BodyResourceEvent {
    pub event_kind: BodyResourceEventKind,
    pub target_id: String,
    pub visibility: BodyResourceVisibility,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BodyResourceEventKind {
    BodyConstraintAdded,
    BodyConstraintChanged,
    BodyConstraintResolved,
    ResourceGained,
    ResourceSpent,
    ResourceLost,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BodyResourceEventRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub event_id: String,
    pub event_kind: BodyResourceEventKind,
    pub target_id: String,
    pub visibility: BodyResourceVisibility,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyResourceEventPlan {
    pub world_id: String,
    pub turn_id: String,
    pub records: Vec<BodyResourceEventRecord>,
}

#[must_use]
pub fn compile_body_resource_packet(snapshot: &TurnSnapshot) -> BodyResourcePacket {
    BodyResourcePacket {
        schema_version: BODY_RESOURCE_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: snapshot.world_id.clone(),
        turn_id: snapshot.turn_id.clone(),
        body_constraints: snapshot
            .protagonist_state
            .body
            .iter()
            .enumerate()
            .map(|(index, body)| BodyConstraint {
                schema_version: BODY_CONSTRAINT_SCHEMA_VERSION.to_owned(),
                constraint_id: format!("body:constraint:{index:02}"),
                visibility: BodyResourceVisibility::PlayerVisible,
                summary: body.clone(),
                severity: infer_body_severity(body),
                source_refs: vec![format!("latest_snapshot.protagonist_state.body[{index}]")],
                scene_pressure_kinds: vec!["body".to_owned()],
            })
            .collect(),
        resources: snapshot
            .protagonist_state
            .inventory
            .iter()
            .enumerate()
            .map(|(index, item)| ResourceItem {
                schema_version: RESOURCE_ITEM_SCHEMA_VERSION.to_owned(),
                resource_id: format!("resource:inventory:{index:02}"),
                visibility: BodyResourceVisibility::PlayerVisible,
                summary: item.clone(),
                resource_kind: infer_resource_kind(item),
                source_refs: vec![format!(
                    "latest_snapshot.protagonist_state.inventory[{index}]"
                )],
            })
            .collect(),
        compiler_policy: BodyResourcePolicy::default(),
    }
}

pub fn prepare_body_resource_event_plan(
    world_id: &str,
    turn_id: &str,
    events: &[BodyResourceEvent],
) -> Result<BodyResourceEventPlan> {
    let recorded_at = Utc::now().to_rfc3339();
    let mut records = Vec::with_capacity(events.len());
    for (index, event) in events.iter().enumerate() {
        validate_body_resource_event(event)
            .map_err(|error| anyhow::anyhow!("invalid body_resource_events[{index}]: {error}"))?;
        records.push(BodyResourceEventRecord {
            schema_version: BODY_RESOURCE_EVENT_SCHEMA_VERSION.to_owned(),
            world_id: world_id.to_owned(),
            turn_id: turn_id.to_owned(),
            event_id: format!("body_resource_event:{turn_id}:{index:02}"),
            event_kind: event.event_kind,
            target_id: event.target_id.trim().to_owned(),
            visibility: event.visibility,
            summary: event.summary.trim().to_owned(),
            evidence_refs: event
                .evidence_refs
                .iter()
                .map(|reference| reference.trim().to_owned())
                .collect(),
            recorded_at: recorded_at.clone(),
        });
    }
    Ok(BodyResourceEventPlan {
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        records,
    })
}

pub fn append_body_resource_event_plan(
    world_dir: &Path,
    plan: &BodyResourceEventPlan,
) -> Result<()> {
    for record in &plan.records {
        append_jsonl(&world_dir.join(BODY_RESOURCE_EVENTS_FILENAME), record)?;
    }
    Ok(())
}

pub fn rebuild_body_resource_state(
    world_dir: &Path,
    base: &BodyResourcePacket,
) -> Result<BodyResourcePacket> {
    let mut state = base.clone();
    let records = load_body_resource_event_records(world_dir)?;
    for record in records {
        apply_body_resource_record(&mut state, &record);
    }
    "materialized_from_snapshot_and_body_resource_events_v1"
        .clone_into(&mut state.compiler_policy.source);
    write_json(&world_dir.join(BODY_RESOURCE_STATE_FILENAME), &state)?;
    Ok(state)
}

pub fn load_body_resource_state(
    world_dir: &Path,
    fallback: BodyResourcePacket,
) -> Result<BodyResourcePacket> {
    let path = world_dir.join(BODY_RESOURCE_STATE_FILENAME);
    if path.exists() {
        return read_json(&path);
    }
    Ok(fallback)
}

fn validate_body_resource_event(event: &BodyResourceEvent) -> Result<()> {
    if event.target_id.trim().is_empty() || event.summary.trim().is_empty() {
        bail!("body/resource event target_id and summary must not be empty");
    }
    if event.evidence_refs.is_empty()
        || event
            .evidence_refs
            .iter()
            .any(|reference| reference.trim().is_empty())
    {
        bail!("body/resource event evidence_refs must contain non-empty visible refs");
    }
    Ok(())
}

fn load_body_resource_event_records(world_dir: &Path) -> Result<Vec<BodyResourceEventRecord>> {
    let path = world_dir.join(BODY_RESOURCE_EVENTS_FILENAME);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)?;
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str::<BodyResourceEventRecord>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn apply_body_resource_record(state: &mut BodyResourcePacket, record: &BodyResourceEventRecord) {
    match record.event_kind {
        BodyResourceEventKind::BodyConstraintAdded
        | BodyResourceEventKind::BodyConstraintChanged => {
            let constraint = BodyConstraint {
                schema_version: BODY_CONSTRAINT_SCHEMA_VERSION.to_owned(),
                constraint_id: record.target_id.clone(),
                visibility: record.visibility,
                summary: record.summary.clone(),
                severity: infer_body_severity(record.summary.as_str()),
                source_refs: vec![format!("body_resource_event:{}", record.event_id)],
                scene_pressure_kinds: vec!["body".to_owned()],
            };
            upsert_body_constraint(&mut state.body_constraints, constraint);
        }
        BodyResourceEventKind::BodyConstraintResolved => {
            state
                .body_constraints
                .retain(|constraint| constraint.constraint_id != record.target_id);
        }
        BodyResourceEventKind::ResourceGained => {
            let resource = ResourceItem {
                schema_version: RESOURCE_ITEM_SCHEMA_VERSION.to_owned(),
                resource_id: record.target_id.clone(),
                visibility: record.visibility,
                summary: record.summary.clone(),
                resource_kind: infer_resource_kind(record.summary.as_str()),
                source_refs: vec![format!("body_resource_event:{}", record.event_id)],
            };
            upsert_resource(&mut state.resources, resource);
        }
        BodyResourceEventKind::ResourceSpent | BodyResourceEventKind::ResourceLost => {
            state
                .resources
                .retain(|resource| resource.resource_id != record.target_id);
        }
    }
}

fn upsert_body_constraint(items: &mut Vec<BodyConstraint>, item: BodyConstraint) {
    if let Some(existing) = items
        .iter_mut()
        .find(|existing| existing.constraint_id == item.constraint_id)
    {
        *existing = item;
    } else {
        items.push(item);
    }
}

fn upsert_resource(items: &mut Vec<ResourceItem>, item: ResourceItem) {
    if let Some(existing) = items
        .iter_mut()
        .find(|existing| existing.resource_id == item.resource_id)
    {
        *existing = item;
    } else {
        items.push(item);
    }
}

fn infer_body_severity(body: &str) -> u8 {
    let lowered = body.to_lowercase();
    if lowered.contains("bleed") || lowered.contains("피") || lowered.contains("severe") {
        4
    } else if lowered.contains("pain") || lowered.contains("아프") || lowered.contains("ache") {
        2
    } else {
        1
    }
}

fn infer_resource_kind(item: &str) -> ResourceKind {
    let lowered = item.to_lowercase();
    if lowered.contains("coin") || lowered.contains("money") || lowered.contains("돈") {
        ResourceKind::Money
    } else if lowered.contains("food") || lowered.contains("bread") || lowered.contains("식량") {
        ResourceKind::Food
    } else if lowered.contains("water") || lowered.contains("물") {
        ResourceKind::Water
    } else if lowered.contains("knife") || lowered.contains("sword") || lowered.contains("검") {
        ResourceKind::Weapon
    } else if lowered.contains("token")
        || lowered.contains("paper")
        || lowered.contains("document")
        || lowered.contains("문서")
        || lowered.contains("패")
    {
        ResourceKind::Document
    } else {
        ResourceKind::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ProtagonistState, TURN_SNAPSHOT_SCHEMA_VERSION, TurnSnapshot};

    #[test]
    fn compiles_body_and_inventory() {
        let snapshot = TurnSnapshot {
            schema_version: TURN_SNAPSHOT_SCHEMA_VERSION.to_owned(),
            world_id: "stw_body".to_owned(),
            session_id: "session".to_owned(),
            turn_id: "turn_0001".to_owned(),
            phase: "choice".to_owned(),
            current_event: None,
            protagonist_state: ProtagonistState {
                location: "place:gate".to_owned(),
                inventory: vec!["wooden entry token".to_owned()],
                body: vec!["left wrist aches".to_owned()],
                mind: Vec::new(),
            },
            open_questions: Vec::new(),
            last_choices: Vec::new(),
        };

        let packet = compile_body_resource_packet(&snapshot);

        assert_eq!(packet.body_constraints.len(), 1);
        assert_eq!(packet.body_constraints[0].severity, 2);
        assert_eq!(packet.resources.len(), 1);
        assert_eq!(packet.resources[0].resource_kind, ResourceKind::Document);
    }
}
