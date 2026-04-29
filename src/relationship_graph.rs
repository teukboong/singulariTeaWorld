use crate::models::RelationshipUpdateRecord;
use serde::{Deserialize, Serialize};

pub const RELATIONSHIP_GRAPH_PACKET_SCHEMA_VERSION: &str = "singulari.relationship_graph_packet.v1";
pub const RELATIONSHIP_EDGE_SCHEMA_VERSION: &str = "singulari.relationship_edge.v1";

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
}
