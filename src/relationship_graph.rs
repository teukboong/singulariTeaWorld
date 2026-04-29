use crate::models::RelationshipUpdateRecord;
use crate::response_context::{AgentContextProjection, AgentRelationshipUpdate, ContextVisibility};
use crate::store::{append_jsonl, read_json, write_json};
use anyhow::{Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const RELATIONSHIP_GRAPH_PACKET_SCHEMA_VERSION: &str = "singulari.relationship_graph_packet.v1";
pub const RELATIONSHIP_EDGE_SCHEMA_VERSION: &str = "singulari.relationship_edge.v1";
pub const RELATIONSHIP_GRAPH_EVENT_SCHEMA_VERSION: &str = "singulari.relationship_graph_event.v1";
pub const RELATIONSHIP_GRAPH_FILENAME: &str = "relationship_graph.json";
pub const RELATIONSHIP_GRAPH_EVENTS_FILENAME: &str = "relationship_graph_events.jsonl";

const RELATIONSHIP_EDGE_BUDGET: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelationshipGraphPacket {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub active_edges: Vec<RelationshipEdge>,
    pub compiler_policy: RelationshipGraphPolicy,
}

impl Default for RelationshipGraphPacket {
    fn default() -> Self {
        Self {
            schema_version: RELATIONSHIP_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            turn_id: String::new(),
            active_edges: Vec::new(),
            compiler_policy: RelationshipGraphPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelationshipEdge {
    pub schema_version: String,
    pub edge_id: String,
    pub source_entity_id: String,
    pub target_entity_id: String,
    pub stance: String,
    pub visibility: String,
    pub visible_summary: String,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default)]
    pub voice_effects: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelationshipGraphEventPlan {
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub records: Vec<RelationshipGraphEventRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelationshipGraphEventRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub event_id: String,
    pub source_entity_id: String,
    pub target_entity_id: String,
    pub relation_kind: String,
    pub visibility: ContextVisibility,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelationshipGraphPolicy {
    pub source: String,
    pub active_edge_budget: usize,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for RelationshipGraphPolicy {
    fn default() -> Self {
        Self {
            source: "compiled_from_relationship_updates_v0".to_owned(),
            active_edge_budget: RELATIONSHIP_EDGE_BUDGET,
            use_rules: vec![
                "Relationship edges affect stance, cooperation, suspicion, debt, and dialogue distance.".to_owned(),
                "Do not treat a relationship edge as hidden motive unless visibility explicitly allows it.".to_owned(),
                "Use visible_summary and stance; do not expose private interpretation.".to_owned(),
            ],
        }
    }
}

#[must_use]
pub fn compile_relationship_graph_packet(
    world_id: &str,
    turn_id: &str,
    updates: &[RelationshipUpdateRecord],
) -> RelationshipGraphPacket {
    let mut active_edges = updates
        .iter()
        .rev()
        .filter(|update| update.visibility == "player_visible")
        .map(relationship_edge)
        .collect::<Vec<_>>();
    active_edges.truncate(RELATIONSHIP_EDGE_BUDGET);
    active_edges.reverse();
    RelationshipGraphPacket {
        schema_version: RELATIONSHIP_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        active_edges,
        compiler_policy: RelationshipGraphPolicy::default(),
    }
}

#[must_use]
pub fn compile_relationship_graph_from_projection(
    world_id: &str,
    turn_id: &str,
    projection: &AgentContextProjection,
    base_updates: &[RelationshipUpdateRecord],
) -> RelationshipGraphPacket {
    if projection.relationship_summaries.is_empty() {
        return compile_relationship_graph_packet(world_id, turn_id, base_updates);
    }
    let mut active_edges = projection
        .relationship_summaries
        .iter()
        .rev()
        .filter(|item| item.visibility == ContextVisibility::PlayerVisible)
        .map(|item| {
            let (source_entity_id, target_entity_id, stance) =
                parse_relationship_projection_target(item.target.as_str());
            RelationshipEdge {
                schema_version: RELATIONSHIP_EDGE_SCHEMA_VERSION.to_owned(),
                edge_id: format!("rel:{source_entity_id}->{target_entity_id}:{stance}"),
                source_entity_id,
                target_entity_id,
                stance,
                visibility: "player_visible".to_owned(),
                visible_summary: item.summary.clone(),
                source_refs: vec![format!("agent_context_event:{}", item.source_event_id)],
                voice_effects: vec![
                    "dialogue stance follows current agent context projection".to_owned(),
                ],
            }
        })
        .collect::<Vec<_>>();
    active_edges.truncate(RELATIONSHIP_EDGE_BUDGET);
    active_edges.reverse();
    RelationshipGraphPacket {
        schema_version: RELATIONSHIP_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        active_edges,
        compiler_policy: RelationshipGraphPolicy {
            source: "compiled_from_agent_context_projection_v1".to_owned(),
            ..RelationshipGraphPolicy::default()
        },
    }
}

/// Validate relationship updates before the turn is advanced.
///
/// # Errors
///
/// Returns an error when an update is missing required fields or visible
/// evidence references.
pub fn prepare_relationship_graph_event_plan(
    world_id: &str,
    turn_id: &str,
    updates: &[AgentRelationshipUpdate],
) -> Result<RelationshipGraphEventPlan> {
    let recorded_at = Utc::now().to_rfc3339();
    let mut records = Vec::new();
    for update in updates {
        validate_relationship_update(update)?;
        records.push(RelationshipGraphEventRecord {
            schema_version: RELATIONSHIP_GRAPH_EVENT_SCHEMA_VERSION.to_owned(),
            world_id: world_id.to_owned(),
            turn_id: turn_id.to_owned(),
            event_id: format!("relationship_graph_event:{turn_id}:{:02}", records.len()),
            source_entity_id: update.source_entity_id.trim().to_owned(),
            target_entity_id: update.target_entity_id.trim().to_owned(),
            relation_kind: update.relation_kind.trim().to_owned(),
            visibility: update.visibility,
            summary: update.summary.trim().to_owned(),
            evidence_refs: update
                .evidence_refs
                .iter()
                .map(|reference| reference.trim().to_owned())
                .collect(),
            recorded_at: recorded_at.clone(),
        });
    }
    Ok(RelationshipGraphEventPlan {
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        records,
    })
}

/// Append prevalidated relationship events to the durable event log.
///
/// # Errors
///
/// Returns an error when the event log cannot be written.
pub fn append_relationship_graph_event_plan(
    world_dir: &Path,
    plan: &RelationshipGraphEventPlan,
) -> Result<()> {
    for record in &plan.records {
        append_jsonl(&world_dir.join(RELATIONSHIP_GRAPH_EVENTS_FILENAME), record)?;
    }
    Ok(())
}

/// Rebuild the materialized relationship graph from the durable event log.
///
/// # Errors
///
/// Returns an error when event records cannot be read or the materialized graph
/// cannot be written.
pub fn rebuild_relationship_graph(
    world_dir: &Path,
    base_packet: &RelationshipGraphPacket,
) -> Result<RelationshipGraphPacket> {
    let records = load_relationship_graph_event_records(world_dir)?;
    let packet = build_relationship_graph_from_events(base_packet, &records);
    write_json(&world_dir.join(RELATIONSHIP_GRAPH_FILENAME), &packet)?;
    Ok(packet)
}

/// Load the materialized relationship graph, or the supplied base packet for
/// legacy worlds that do not have the dedicated file yet.
///
/// # Errors
///
/// Returns an error when an existing materialized graph cannot be parsed.
pub fn load_relationship_graph_state(
    world_dir: &Path,
    base_packet: RelationshipGraphPacket,
) -> Result<RelationshipGraphPacket> {
    let path = world_dir.join(RELATIONSHIP_GRAPH_FILENAME);
    if path.is_file() {
        return read_json(&path);
    }
    Ok(base_packet)
}

#[must_use]
pub fn build_relationship_graph_from_events(
    base_packet: &RelationshipGraphPacket,
    records: &[RelationshipGraphEventRecord],
) -> RelationshipGraphPacket {
    let mut active_edges = base_packet.active_edges.clone();
    for record in records
        .iter()
        .filter(|record| record.visibility == ContextVisibility::PlayerVisible)
    {
        let edge = relationship_edge_from_event(record);
        if let Some(existing) = active_edges
            .iter_mut()
            .find(|existing| existing.edge_id == edge.edge_id)
        {
            *existing = edge;
        } else {
            active_edges.push(edge);
        }
    }
    if active_edges.len() > RELATIONSHIP_EDGE_BUDGET {
        let overflow = active_edges.len() - RELATIONSHIP_EDGE_BUDGET;
        active_edges.drain(0..overflow);
    }
    RelationshipGraphPacket {
        schema_version: RELATIONSHIP_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: base_packet.world_id.clone(),
        turn_id: records.last().map_or_else(
            || base_packet.turn_id.clone(),
            |record| record.turn_id.clone(),
        ),
        active_edges,
        compiler_policy: RelationshipGraphPolicy {
            source: "materialized_from_relationship_graph_events_v1".to_owned(),
            ..RelationshipGraphPolicy::default()
        },
    }
}

/// Load relationship graph events from the durable event log.
///
/// # Errors
///
/// Returns an error when the event log cannot be read or contains malformed
/// JSON lines.
pub fn load_relationship_graph_event_records(
    world_dir: &Path,
) -> Result<Vec<RelationshipGraphEventRecord>> {
    let path = world_dir.join(RELATIONSHIP_GRAPH_EVENTS_FILENAME);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|error| anyhow::anyhow!("failed to read {}: {error}", path.display()))?;
    raw.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str::<RelationshipGraphEventRecord>(line).map_err(|error| {
                anyhow::anyhow!(
                    "failed to parse {} line {} as RelationshipGraphEventRecord: {error}",
                    path.display(),
                    index + 1
                )
            })
        })
        .collect()
}

fn parse_relationship_projection_target(target: &str) -> (String, String, String) {
    let Some((source_entity_id, rest)) = target.split_once("->") else {
        return (
            target.to_owned(),
            "unknown".to_owned(),
            "context".to_owned(),
        );
    };
    let Some((target_entity_id, stance)) = rest.rsplit_once(':') else {
        return (
            source_entity_id.to_owned(),
            rest.to_owned(),
            "context".to_owned(),
        );
    };
    (
        source_entity_id.to_owned(),
        target_entity_id.to_owned(),
        stance.to_owned(),
    )
}

fn relationship_edge(update: &RelationshipUpdateRecord) -> RelationshipEdge {
    RelationshipEdge {
        schema_version: RELATIONSHIP_EDGE_SCHEMA_VERSION.to_owned(),
        edge_id: format!(
            "rel:{}->{}:{}",
            update.source_entity_id, update.target_entity_id, update.relation_kind
        ),
        source_entity_id: update.source_entity_id.clone(),
        target_entity_id: update.target_entity_id.clone(),
        stance: update.relation_kind.clone(),
        visibility: update.visibility.clone(),
        visible_summary: update.summary.clone(),
        source_refs: vec![format!("relationship_update:{}", update.update_id)],
        voice_effects: vec![format!(
            "dialogue stance follows relation_kind={}",
            update.relation_kind
        )],
    }
}

fn relationship_edge_from_event(record: &RelationshipGraphEventRecord) -> RelationshipEdge {
    RelationshipEdge {
        schema_version: RELATIONSHIP_EDGE_SCHEMA_VERSION.to_owned(),
        edge_id: format!(
            "rel:{}->{}:{}",
            record.source_entity_id, record.target_entity_id, record.relation_kind
        ),
        source_entity_id: record.source_entity_id.clone(),
        target_entity_id: record.target_entity_id.clone(),
        stance: record.relation_kind.clone(),
        visibility: "player_visible".to_owned(),
        visible_summary: record.summary.clone(),
        source_refs: vec![format!("relationship_graph_event:{}", record.event_id)],
        voice_effects: vec![format!(
            "dialogue stance follows relation_kind={}",
            record.relation_kind
        )],
    }
}

fn validate_relationship_update(update: &AgentRelationshipUpdate) -> Result<()> {
    let fields = [
        update.source_entity_id.as_str(),
        update.target_entity_id.as_str(),
        update.relation_kind.as_str(),
        update.summary.as_str(),
    ];
    if fields.iter().any(|field| field.trim().is_empty()) {
        bail!("relationship_updates contains an empty required field");
    }
    if update.evidence_refs.is_empty()
        || update
            .evidence_refs
            .iter()
            .any(|reference| reference.trim().is_empty())
    {
        bail!("relationship_updates evidence_refs must contain non-empty visible refs");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_player_visible_relationship_updates_as_edges() {
        let updates = vec![RelationshipUpdateRecord {
            update_id: "rel_update_1".to_owned(),
            world_id: "stw_rel".to_owned(),
            turn_id: "turn_0001".to_owned(),
            source_entity_id: "char:guard".to_owned(),
            target_entity_id: "char:protagonist".to_owned(),
            relation_kind: "procedural_suspicion".to_owned(),
            visibility: "player_visible".to_owned(),
            summary: "the guard treats the protagonist as procedurally suspicious".to_owned(),
            source_event_id: "evt_1".to_owned(),
            created_at: "2026-04-29T00:00:00Z".to_owned(),
        }];

        let packet = compile_relationship_graph_packet("stw_rel", "turn_0002", &updates);

        assert_eq!(packet.active_edges.len(), 1);
        assert_eq!(packet.active_edges[0].stance, "procedural_suspicion");
        assert_eq!(
            packet.active_edges[0].edge_id,
            "rel:char:guard->char:protagonist:procedural_suspicion"
        );
    }

    #[test]
    fn projection_overrides_recent_relationship_updates() {
        let projection = AgentContextProjection {
            world_id: "stw_rel".to_owned(),
            turn_id: "turn_0002".to_owned(),
            relationship_summaries: vec![crate::response_context::AgentContextProjectionItem {
                target: "char:a->char:b:trust".to_owned(),
                summary: "a trusts b after the scene".to_owned(),
                source_event_id: "ctx_1".to_owned(),
                turn_id: "turn_0002".to_owned(),
                visibility: ContextVisibility::PlayerVisible,
            }],
            ..AgentContextProjection::default()
        };

        let packet =
            compile_relationship_graph_from_projection("stw_rel", "turn_0002", &projection, &[]);

        assert_eq!(
            packet.compiler_policy.source,
            "compiled_from_agent_context_projection_v1"
        );
        assert_eq!(packet.active_edges[0].source_entity_id, "char:a");
        assert_eq!(packet.active_edges[0].target_entity_id, "char:b");
        assert_eq!(packet.active_edges[0].stance, "trust");
    }

    #[test]
    fn materializes_relationship_events_over_base_packet() -> Result<()> {
        let base_packet = RelationshipGraphPacket {
            world_id: "stw_rel".to_owned(),
            turn_id: "turn_0001".to_owned(),
            ..RelationshipGraphPacket::default()
        };
        let plan = prepare_relationship_graph_event_plan(
            "stw_rel",
            "turn_0002",
            &[AgentRelationshipUpdate {
                source_entity_id: "char:a".to_owned(),
                target_entity_id: "char:b".to_owned(),
                relation_kind: "cautious_trust".to_owned(),
                visibility: ContextVisibility::PlayerVisible,
                summary: "a now trusts b with caution".to_owned(),
                evidence_refs: vec!["visible_scene.text_blocks[0]".to_owned()],
            }],
        )?;

        let packet = build_relationship_graph_from_events(&base_packet, &plan.records);

        assert_eq!(packet.active_edges.len(), 1);
        assert_eq!(packet.active_edges[0].stance, "cautious_trust");
        assert_eq!(
            packet.compiler_policy.source,
            "materialized_from_relationship_graph_events_v1"
        );
        Ok(())
    }
}
