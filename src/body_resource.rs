use crate::models::TurnSnapshot;
use serde::{Deserialize, Serialize};

pub const BODY_RESOURCE_PACKET_SCHEMA_VERSION: &str = "singulari.body_resource_packet.v1";
pub const BODY_CONSTRAINT_SCHEMA_VERSION: &str = "singulari.body_constraint.v1";
pub const RESOURCE_ITEM_SCHEMA_VERSION: &str = "singulari.resource_item.v1";

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
