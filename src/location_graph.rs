// Projection APIs return anyhow::Result with per-call path/context details; the
// Rustdoc error lists would duplicate those local error messages.
#![allow(clippy::missing_errors_doc)]

use crate::models::{EntityRecords, PlaceRecord, TurnSnapshot};
use crate::store::{append_jsonl, read_json, write_json};
use anyhow::{Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const LOCATION_GRAPH_PACKET_SCHEMA_VERSION: &str = "singulari.location_graph_packet.v1";
pub const LOCATION_NODE_SCHEMA_VERSION: &str = "singulari.location_node.v1";
pub const LOCATION_EVENT_SCHEMA_VERSION: &str = "singulari.location_event.v1";
pub const LOCATION_EVENTS_FILENAME: &str = "location_events.jsonl";
pub const LOCATION_GRAPH_FILENAME: &str = "location_graph.json";

const NEARBY_LOCATION_BUDGET: usize = 3;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocationGraphPacket {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_location: Option<LocationNode>,
    #[serde(default)]
    pub known_nearby_locations: Vec<LocationNode>,
    pub compiler_policy: LocationGraphPolicy,
}

impl Default for LocationGraphPacket {
    fn default() -> Self {
        Self {
            schema_version: LOCATION_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            turn_id: String::new(),
            current_location: None,
            known_nearby_locations: Vec::new(),
            compiler_policy: LocationGraphPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocationNode {
    pub schema_version: String,
    pub location_id: String,
    pub name: String,
    pub knowledge_state: LocationKnowledgeState,
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default)]
    pub source_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocationGraphPolicy {
    pub source: String,
    pub nearby_location_budget: usize,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for LocationGraphPolicy {
    fn default() -> Self {
        Self {
            source: "compiled_from_entity_places_v0".to_owned(),
            nearby_location_budget: NEARBY_LOCATION_BUDGET,
            use_rules: vec![
                "Movement choices should come from current or known nearby locations.".to_owned(),
                "Do not invent exits, towns, or landmarks unless visible evidence opens a discovery action.".to_owned(),
                "Location notes may create sensory continuity but not hidden lore.".to_owned(),
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocationKnowledgeState {
    Known,
    Visited,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocationEvent {
    pub event_kind: LocationEventKind,
    pub location_id: String,
    pub name: String,
    pub knowledge_state: LocationKnowledgeState,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocationEventKind {
    Discovered,
    Visited,
    Updated,
    RouteOpened,
    RouteBlocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocationEventRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub event_id: String,
    pub event_kind: LocationEventKind,
    pub location_id: String,
    pub name: String,
    pub knowledge_state: LocationKnowledgeState,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationEventPlan {
    pub world_id: String,
    pub turn_id: String,
    pub records: Vec<LocationEventRecord>,
}

#[must_use]
pub fn compile_location_graph_packet(
    snapshot: &TurnSnapshot,
    entities: &EntityRecords,
) -> LocationGraphPacket {
    let current_location = entities
        .places
        .iter()
        .find(|place| place.id == snapshot.protagonist_state.location)
        .map(|place| location_node(place, LocationKnowledgeState::Visited));
    let mut known_nearby_locations = entities
        .places
        .iter()
        .filter(|place| place.id != snapshot.protagonist_state.location)
        .filter(|place| place.known_to_protagonist)
        .take(NEARBY_LOCATION_BUDGET)
        .map(|place| location_node(place, LocationKnowledgeState::Known))
        .collect::<Vec<_>>();
    known_nearby_locations.shrink_to_fit();
    LocationGraphPacket {
        schema_version: LOCATION_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: snapshot.world_id.clone(),
        turn_id: snapshot.turn_id.clone(),
        current_location,
        known_nearby_locations,
        compiler_policy: LocationGraphPolicy::default(),
    }
}

pub fn prepare_location_event_plan(
    world_id: &str,
    turn_id: &str,
    events: &[LocationEvent],
) -> Result<LocationEventPlan> {
    let recorded_at = Utc::now().to_rfc3339();
    let mut records = Vec::with_capacity(events.len());
    for (index, event) in events.iter().enumerate() {
        validate_location_event(event)
            .map_err(|error| anyhow::anyhow!("invalid location_events[{index}]: {error}"))?;
        records.push(LocationEventRecord {
            schema_version: LOCATION_EVENT_SCHEMA_VERSION.to_owned(),
            world_id: world_id.to_owned(),
            turn_id: turn_id.to_owned(),
            event_id: format!("location_event:{turn_id}:{index:02}"),
            event_kind: event.event_kind,
            location_id: event.location_id.trim().to_owned(),
            name: event.name.trim().to_owned(),
            knowledge_state: event.knowledge_state,
            summary: event.summary.trim().to_owned(),
            evidence_refs: event
                .evidence_refs
                .iter()
                .map(|reference| reference.trim().to_owned())
                .collect(),
            recorded_at: recorded_at.clone(),
        });
    }
    Ok(LocationEventPlan {
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        records,
    })
}

pub fn append_location_event_plan(world_dir: &Path, plan: &LocationEventPlan) -> Result<()> {
    for record in &plan.records {
        append_jsonl(&world_dir.join(LOCATION_EVENTS_FILENAME), record)?;
    }
    Ok(())
}

pub fn rebuild_location_graph(
    world_dir: &Path,
    base: &LocationGraphPacket,
) -> Result<LocationGraphPacket> {
    let mut graph = base.clone();
    for record in load_location_event_records(world_dir)? {
        apply_location_record(&mut graph, &record);
    }
    "materialized_from_entities_and_location_events_v1"
        .clone_into(&mut graph.compiler_policy.source);
    write_json(&world_dir.join(LOCATION_GRAPH_FILENAME), &graph)?;
    Ok(graph)
}

pub fn load_location_graph_state(
    world_dir: &Path,
    fallback: LocationGraphPacket,
) -> Result<LocationGraphPacket> {
    let path = world_dir.join(LOCATION_GRAPH_FILENAME);
    if path.exists() {
        return read_json(&path);
    }
    Ok(fallback)
}

fn validate_location_event(event: &LocationEvent) -> Result<()> {
    if event.location_id.trim().is_empty()
        || event.name.trim().is_empty()
        || event.summary.trim().is_empty()
    {
        bail!("location event location_id, name, and summary must not be empty");
    }
    if event.evidence_refs.is_empty()
        || event
            .evidence_refs
            .iter()
            .any(|reference| reference.trim().is_empty())
    {
        bail!("location event evidence_refs must contain non-empty visible refs");
    }
    Ok(())
}

fn load_location_event_records(world_dir: &Path) -> Result<Vec<LocationEventRecord>> {
    let path = world_dir.join(LOCATION_EVENTS_FILENAME);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)?;
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str::<LocationEventRecord>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn apply_location_record(graph: &mut LocationGraphPacket, record: &LocationEventRecord) {
    let node = LocationNode {
        schema_version: LOCATION_NODE_SCHEMA_VERSION.to_owned(),
        location_id: record.location_id.clone(),
        name: record.name.clone(),
        knowledge_state: record.knowledge_state,
        notes: vec![record.summary.clone()],
        source_refs: vec![format!("location_event:{}", record.event_id)],
    };
    if matches!(record.event_kind, LocationEventKind::Visited) {
        graph.current_location = Some(node);
        return;
    }
    if let Some(existing) = graph
        .known_nearby_locations
        .iter_mut()
        .find(|existing| existing.location_id == node.location_id)
    {
        *existing = node;
    } else {
        graph.known_nearby_locations.push(node);
    }
    graph
        .known_nearby_locations
        .truncate(NEARBY_LOCATION_BUDGET);
}

fn location_node(place: &PlaceRecord, knowledge_state: LocationKnowledgeState) -> LocationNode {
    LocationNode {
        schema_version: LOCATION_NODE_SCHEMA_VERSION.to_owned(),
        location_id: place.id.clone(),
        name: place.name.clone(),
        knowledge_state,
        notes: place.notes.clone(),
        source_refs: vec![format!("entities.places:{}", place.id)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        EntityRecords, ProtagonistState, TURN_SNAPSHOT_SCHEMA_VERSION, TurnSnapshot,
    };

    #[test]
    fn compiles_current_and_known_nearby_locations() {
        let snapshot = TurnSnapshot {
            schema_version: TURN_SNAPSHOT_SCHEMA_VERSION.to_owned(),
            world_id: "stw_location".to_owned(),
            session_id: "session".to_owned(),
            turn_id: "turn_0001".to_owned(),
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
        let entities = EntityRecords {
            schema_version: "singulari.entities.v1".to_owned(),
            world_id: "stw_location".to_owned(),
            characters: Vec::new(),
            places: vec![
                PlaceRecord {
                    id: "place:gate".to_owned(),
                    name: "West Gate".to_owned(),
                    coordinates: None,
                    known_to_protagonist: true,
                    notes: vec!["wet stone arch".to_owned()],
                },
                PlaceRecord {
                    id: "place:road".to_owned(),
                    name: "Old Road".to_owned(),
                    coordinates: None,
                    known_to_protagonist: true,
                    notes: Vec::new(),
                },
            ],
            factions: Vec::new(),
            items: Vec::new(),
            concepts: Vec::new(),
        };

        let packet = compile_location_graph_packet(&snapshot, &entities);

        assert_eq!(
            packet
                .current_location
                .as_ref()
                .map(|location| location.location_id.as_str()),
            Some("place:gate")
        );
        assert_eq!(packet.known_nearby_locations.len(), 1);
        assert_eq!(
            packet.known_nearby_locations[0].knowledge_state,
            LocationKnowledgeState::Known
        );
    }
}
