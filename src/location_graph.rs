use crate::models::{EntityRecords, PlaceRecord, TurnSnapshot};
use serde::{Deserialize, Serialize};

pub const LOCATION_GRAPH_PACKET_SCHEMA_VERSION: &str = "singulari.location_graph_packet.v1";
pub const LOCATION_NODE_SCHEMA_VERSION: &str = "singulari.location_node.v1";

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
