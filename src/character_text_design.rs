use crate::models::{CharacterRecord, CharacterVoiceAnchor, EntityRecords};
use crate::response_context::{
    AgentCharacterTextDesignUpdate, AgentContextProjection, ContextVisibility,
};
use crate::store::{append_jsonl, read_json, write_json};
use anyhow::{Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const CHARACTER_TEXT_DESIGN_PACKET_SCHEMA_VERSION: &str =
    "singulari.character_text_design_packet.v1";
pub const CHARACTER_TEXT_DESIGN_SCHEMA_VERSION: &str = "singulari.character_text_design.v1";
pub const CHARACTER_TEXT_DESIGN_EVENT_SCHEMA_VERSION: &str =
    "singulari.character_text_design_event.v1";
pub const CHARACTER_TEXT_DESIGN_FILENAME: &str = "character_text_design.json";
pub const CHARACTER_TEXT_DESIGN_EVENTS_FILENAME: &str = "character_text_design_events.jsonl";

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
pub struct CharacterTextDesignEventPlan {
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub records: Vec<CharacterTextDesignEventRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CharacterTextDesignEventRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub event_id: String,
    pub character_id: String,
    pub speech_pattern: String,
    pub gesture_pattern: String,
    pub drift_note: String,
    pub visibility: ContextVisibility,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub recorded_at: String,
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

/// Validate character text-design updates before the turn is advanced.
///
/// # Errors
///
/// Returns an error when an update is missing required fields or visible
/// evidence references.
pub fn prepare_character_text_design_event_plan(
    world_id: &str,
    turn_id: &str,
    updates: &[AgentCharacterTextDesignUpdate],
) -> Result<CharacterTextDesignEventPlan> {
    let recorded_at = Utc::now().to_rfc3339();
    let mut records = Vec::new();
    for update in updates {
        validate_character_text_design_update(update)?;
        records.push(CharacterTextDesignEventRecord {
            schema_version: CHARACTER_TEXT_DESIGN_EVENT_SCHEMA_VERSION.to_owned(),
            world_id: world_id.to_owned(),
            turn_id: turn_id.to_owned(),
            event_id: format!("character_text_design_event:{turn_id}:{:02}", records.len()),
            character_id: update.character_id.trim().to_owned(),
            speech_pattern: update.speech_pattern.trim().to_owned(),
            gesture_pattern: update.gesture_pattern.trim().to_owned(),
            drift_note: update.drift_note.trim().to_owned(),
            visibility: update.visibility,
            evidence_refs: update
                .evidence_refs
                .iter()
                .map(|reference| reference.trim().to_owned())
                .collect(),
            recorded_at: recorded_at.clone(),
        });
    }
    Ok(CharacterTextDesignEventPlan {
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        records,
    })
}

/// Append prevalidated character text-design events to the durable event log.
///
/// # Errors
///
/// Returns an error when the event log cannot be written.
pub fn append_character_text_design_event_plan(
    world_dir: &Path,
    plan: &CharacterTextDesignEventPlan,
) -> Result<()> {
    for record in &plan.records {
        append_jsonl(
            &world_dir.join(CHARACTER_TEXT_DESIGN_EVENTS_FILENAME),
            record,
        )?;
    }
    Ok(())
}

/// Rebuild the materialized character text-design packet from the event log.
///
/// # Errors
///
/// Returns an error when event records cannot be read or the materialized packet
/// cannot be written.
pub fn rebuild_character_text_design(
    world_dir: &Path,
    base_packet: &CharacterTextDesignPacket,
) -> Result<CharacterTextDesignPacket> {
    let records = load_character_text_design_event_records(world_dir)?;
    let packet = build_character_text_design_from_events(base_packet, &records);
    write_json(&world_dir.join(CHARACTER_TEXT_DESIGN_FILENAME), &packet)?;
    Ok(packet)
}

/// Load the materialized character text-design packet, or the supplied base
/// packet for legacy worlds that do not have the dedicated file yet.
///
/// # Errors
///
/// Returns an error when an existing materialized packet cannot be parsed.
pub fn load_character_text_design_state(
    world_dir: &Path,
    base_packet: CharacterTextDesignPacket,
) -> Result<CharacterTextDesignPacket> {
    let path = world_dir.join(CHARACTER_TEXT_DESIGN_FILENAME);
    if path.is_file() {
        return read_json(&path);
    }
    Ok(base_packet)
}

#[must_use]
pub fn build_character_text_design_from_events(
    base_packet: &CharacterTextDesignPacket,
    records: &[CharacterTextDesignEventRecord],
) -> CharacterTextDesignPacket {
    let mut active_designs = base_packet.active_designs.clone();
    for record in records
        .iter()
        .filter(|record| record.visibility == ContextVisibility::PlayerVisible)
    {
        let design = character_text_design_from_event(record, &active_designs);
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
        world_id: base_packet.world_id.clone(),
        active_designs,
        compiler_policy: CharacterTextDesignPolicy {
            source: "materialized_from_character_text_design_events_v1".to_owned(),
            ..CharacterTextDesignPolicy::default()
        },
    }
}

/// Load character text-design event records from the durable event log.
///
/// # Errors
///
/// Returns an error when the event log cannot be read or contains malformed
/// JSON lines.
pub fn load_character_text_design_event_records(
    world_dir: &Path,
) -> Result<Vec<CharacterTextDesignEventRecord>> {
    let path = world_dir.join(CHARACTER_TEXT_DESIGN_EVENTS_FILENAME);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|error| anyhow::anyhow!("failed to read {}: {error}", path.display()))?;
    raw.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str::<CharacterTextDesignEventRecord>(line).map_err(|error| {
                anyhow::anyhow!(
                    "failed to parse {} line {} as CharacterTextDesignEventRecord: {error}",
                    path.display(),
                    index + 1
                )
            })
        })
        .collect()
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

fn character_text_design_from_event(
    record: &CharacterTextDesignEventRecord,
    current_designs: &[CharacterTextDesign],
) -> CharacterTextDesign {
    let existing = current_designs
        .iter()
        .find(|design| design.entity_id == record.character_id);
    CharacterTextDesign {
        schema_version: CHARACTER_TEXT_DESIGN_SCHEMA_VERSION.to_owned(),
        entity_id: record.character_id.clone(),
        visible_name: existing.map_or_else(
            || record.character_id.clone(),
            |design| design.visible_name.clone(),
        ),
        role: existing.map_or_else(
            || "context-designed character".to_owned(),
            |design| design.role.clone(),
        ),
        visibility: "player_visible".to_owned(),
        speech: vec![record.speech_pattern.clone()],
        endings: existing.map_or_else(Vec::new, |design| design.endings.clone()),
        tone: existing.map_or_else(Vec::new, |design| design.tone.clone()),
        gestures: vec![record.gesture_pattern.clone()],
        habits: existing.map_or_else(Vec::new, |design| design.habits.clone()),
        drift: vec![record.drift_note.clone()],
        source_refs: vec![format!("character_text_design_event:{}", record.event_id)],
    }
}

fn validate_character_text_design_update(update: &AgentCharacterTextDesignUpdate) -> Result<()> {
    let fields = [
        update.character_id.as_str(),
        update.speech_pattern.as_str(),
        update.gesture_pattern.as_str(),
        update.drift_note.as_str(),
    ];
    if fields.iter().any(|field| field.trim().is_empty()) {
        bail!("character_text_design_updates contains an empty required field");
    }
    if update.evidence_refs.is_empty()
        || update
            .evidence_refs
            .iter()
            .any(|reference| reference.trim().is_empty())
    {
        bail!("character_text_design_updates evidence_refs must contain non-empty visible refs");
    }
    Ok(())
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

    #[test]
    fn materializes_character_text_design_events_over_base_packet() -> Result<()> {
        let base_packet = CharacterTextDesignPacket {
            world_id: "stw_voice".to_owned(),
            active_designs: vec![CharacterTextDesign {
                schema_version: CHARACTER_TEXT_DESIGN_SCHEMA_VERSION.to_owned(),
                entity_id: "char:guard".to_owned(),
                visible_name: "Gate Guard".to_owned(),
                role: "guard".to_owned(),
                visibility: "player_visible".to_owned(),
                speech: vec!["brief questions".to_owned()],
                endings: Vec::new(),
                tone: Vec::new(),
                gestures: Vec::new(),
                habits: Vec::new(),
                drift: Vec::new(),
                source_refs: Vec::new(),
            }],
            ..CharacterTextDesignPacket::default()
        };
        let plan = prepare_character_text_design_event_plan(
            "stw_voice",
            "turn_0002",
            &[AgentCharacterTextDesignUpdate {
                character_id: "char:guard".to_owned(),
                speech_pattern: "short clipped warnings".to_owned(),
                gesture_pattern: "keeps one hand on the gate chain".to_owned(),
                drift_note: "warms only after proven caution".to_owned(),
                visibility: ContextVisibility::PlayerVisible,
                evidence_refs: vec!["visible_scene.text_blocks[0]".to_owned()],
            }],
        )?;

        let packet = build_character_text_design_from_events(&base_packet, &plan.records);

        assert_eq!(packet.active_designs.len(), 1);
        assert_eq!(packet.active_designs[0].visible_name, "Gate Guard");
        assert_eq!(
            packet.compiler_policy.source,
            "materialized_from_character_text_design_events_v1"
        );
        Ok(())
    }
}
