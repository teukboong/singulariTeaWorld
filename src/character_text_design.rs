use crate::models::{CharacterRecord, CharacterVoiceAnchor, EntityRecords};
use crate::response_context::{AgentContextProjection, ContextVisibility};
use serde::{Deserialize, Serialize};

pub const CHARACTER_TEXT_DESIGN_PACKET_SCHEMA_VERSION: &str =
    "singulari.character_text_design_packet.v1";
pub const CHARACTER_TEXT_DESIGN_SCHEMA_VERSION: &str = "singulari.character_text_design.v1";

const CHARACTER_TEXT_DESIGN_BUDGET: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CharacterTextDesignPacket {
    pub schema_version: String,
    pub world_id: String,
    #[serde(default)]
    pub active_designs: Vec<CharacterTextDesign>,
    pub compiler_policy: CharacterTextDesignPolicy,
}

impl Default for CharacterTextDesignPacket {
    fn default() -> Self {
        Self {
            schema_version: CHARACTER_TEXT_DESIGN_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            active_designs: Vec::new(),
            compiler_policy: CharacterTextDesignPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CharacterTextDesign {
    pub schema_version: String,
    pub entity_id: String,
    pub visible_name: String,
    pub role: String,
    pub visibility: String,
    pub speech: Vec<String>,
    pub endings: Vec<String>,
    pub tone: Vec<String>,
    pub gestures: Vec<String>,
    pub habits: Vec<String>,
    pub drift: Vec<String>,
    #[serde(default)]
    pub source_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CharacterTextDesignPolicy {
    pub source: String,
    pub active_design_budget: usize,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for CharacterTextDesignPolicy {
    fn default() -> Self {
        Self {
            source: "compiled_from_entity_voice_anchors_v0".to_owned(),
            active_design_budget: CHARACTER_TEXT_DESIGN_BUDGET,
            use_rules: vec![
                "Character text design controls speech, endings, tone, gestures, habits, and drift.".to_owned(),
                "Do not let prose style overwrite character-specific voice.".to_owned(),
                "Relationship stance may modify delivery, but does not create base voice.".to_owned(),
            ],
        }
    }
}

#[must_use]
pub fn compile_character_text_design_packet(entities: &EntityRecords) -> CharacterTextDesignPacket {
    compile_character_text_design_with_projection(entities, &AgentContextProjection::default())
}

#[must_use]
pub fn compile_character_text_design_with_projection(
    entities: &EntityRecords,
    projection: &AgentContextProjection,
) -> CharacterTextDesignPacket {
    let mut active_designs = entities
        .characters
        .iter()
        .filter(|character| !character.voice_anchor.is_empty())
        .take(CHARACTER_TEXT_DESIGN_BUDGET)
        .map(character_text_design)
        .collect::<Vec<_>>();
    for item in projection
        .character_text_design_summaries
        .iter()
        .filter(|item| item.visibility == ContextVisibility::PlayerVisible)
    {
        let design = projected_character_text_design(item);
        if let Some(existing) = active_designs
            .iter_mut()
            .find(|existing| existing.entity_id == design.entity_id)
        {
            *existing = design;
        } else {
            active_designs.push(design);
        }
    }
    active_designs.truncate(CHARACTER_TEXT_DESIGN_BUDGET);
    CharacterTextDesignPacket {
        schema_version: CHARACTER_TEXT_DESIGN_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: entities.world_id.clone(),
        active_designs,
        compiler_policy: CharacterTextDesignPolicy {
            source: if projection.character_text_design_summaries.is_empty() {
                CharacterTextDesignPolicy::default().source
            } else {
                "compiled_from_entity_voice_anchors_and_agent_context_projection_v1".to_owned()
            },
            ..CharacterTextDesignPolicy::default()
        },
    }
}

fn projected_character_text_design(
    item: &crate::response_context::AgentContextProjectionItem,
) -> CharacterTextDesign {
    CharacterTextDesign {
        schema_version: CHARACTER_TEXT_DESIGN_SCHEMA_VERSION.to_owned(),
        entity_id: item.target.clone(),
        visible_name: item.target.clone(),
        role: "context-projected character".to_owned(),
        visibility: "player_visible".to_owned(),
        speech: vec![item.summary.clone()],
        endings: Vec::new(),
        tone: Vec::new(),
        gestures: vec![item.summary.clone()],
        habits: Vec::new(),
        drift: vec![item.summary.clone()],
        source_refs: vec![format!("agent_context_event:{}", item.source_event_id)],
    }
}

fn character_text_design(character: &CharacterRecord) -> CharacterTextDesign {
    let CharacterVoiceAnchor {
        speech,
        endings,
        tone,
        gestures,
        habits,
        drift,
    } = character.voice_anchor.clone();
    CharacterTextDesign {
        schema_version: CHARACTER_TEXT_DESIGN_SCHEMA_VERSION.to_owned(),
        entity_id: character.id.clone(),
        visible_name: character.name.visible.clone(),
        role: character.role.clone(),
        visibility: character.knowledge_state.clone(),
        speech,
        endings,
        tone,
        gestures,
        habits,
        drift,
        source_refs: vec![format!("entities.characters:{}", character.id)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        BodyNeeds, CharacterBody, CharacterRecord, CharacterVoiceAnchor, EntityName, EntityRecords,
        TraitSet,
    };

    #[test]
    fn compiles_voice_anchor_as_text_design() {
        let entities = EntityRecords {
            schema_version: "singulari.entities.v1".to_owned(),
            world_id: "stw_voice".to_owned(),
            characters: vec![CharacterRecord {
                id: "char:guard".to_owned(),
                name: EntityName {
                    visible: "Gate Guard".to_owned(),
                    native: None,
                },
                role: "guard".to_owned(),
                knowledge_state: "player_visible".to_owned(),
                traits: TraitSet {
                    confirmed: Vec::new(),
                    rumored: Vec::new(),
                    hidden: Vec::new(),
                },
                voice_anchor: CharacterVoiceAnchor {
                    speech: vec!["short procedural questions".to_owned()],
                    endings: Vec::new(),
                    tone: Vec::new(),
                    gestures: Vec::new(),
                    habits: Vec::new(),
                    drift: Vec::new(),
                },
                body: CharacterBody {
                    injuries: Vec::new(),
                    needs: BodyNeeds {
                        hunger: "stable".to_owned(),
                        thirst: "stable".to_owned(),
                        fatigue: "stable".to_owned(),
                    },
                },
                history: Vec::new(),
                relationships: Vec::new(),
            }],
            places: Vec::new(),
            factions: Vec::new(),
            items: Vec::new(),
            concepts: Vec::new(),
        };

        let packet = compile_character_text_design_packet(&entities);

        assert_eq!(packet.active_designs.len(), 1);
        assert_eq!(packet.active_designs[0].entity_id, "char:guard");
        assert_eq!(
            packet.active_designs[0].speech,
            vec!["short procedural questions".to_owned()]
        );
    }

    #[test]
    fn projection_overrides_character_text_design() {
        let mut entities = EntityRecords {
            schema_version: "singulari.entities.v1".to_owned(),
            world_id: "stw_voice".to_owned(),
            characters: Vec::new(),
            places: Vec::new(),
            factions: Vec::new(),
            items: Vec::new(),
            concepts: Vec::new(),
        };
        entities.characters.push(CharacterRecord {
            id: "char:guard".to_owned(),
            name: EntityName {
                visible: "Gate Guard".to_owned(),
                native: None,
            },
            role: "guard".to_owned(),
            knowledge_state: "player_visible".to_owned(),
            traits: TraitSet {
                confirmed: Vec::new(),
                rumored: Vec::new(),
                hidden: Vec::new(),
            },
            voice_anchor: CharacterVoiceAnchor::default(),
            body: CharacterBody {
                injuries: Vec::new(),
                needs: BodyNeeds {
                    hunger: "stable".to_owned(),
                    thirst: "stable".to_owned(),
                    fatigue: "stable".to_owned(),
                },
            },
            history: Vec::new(),
            relationships: Vec::new(),
        });
        let projection = AgentContextProjection {
            world_id: "stw_voice".to_owned(),
            character_text_design_summaries: vec![
                crate::response_context::AgentContextProjectionItem {
                    target: "char:guard".to_owned(),
                    summary: "short clipped warnings".to_owned(),
                    source_event_id: "ctx_1".to_owned(),
                    turn_id: "turn_0002".to_owned(),
                    visibility: ContextVisibility::PlayerVisible,
                },
            ],
            ..AgentContextProjection::default()
        };

        let packet = compile_character_text_design_with_projection(&entities, &projection);

        assert_eq!(packet.active_designs.len(), 1);
        assert_eq!(packet.active_designs[0].entity_id, "char:guard");
        assert!(packet.active_designs[0].speech[0].contains("short clipped"));
    }
}
