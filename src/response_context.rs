// Projection APIs return anyhow::Result with per-call path/context details; the
// Rustdoc error lists would duplicate those local error messages.
#![allow(clippy::missing_errors_doc)]
// Event compiler functions keep the schema shape visible in one place so the
// WebGPT response contract stays auditable.
#![allow(clippy::too_many_lines, clippy::too_many_arguments)]

use crate::store::{append_jsonl, read_json, write_json};
use anyhow::{Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const AGENT_CONTEXT_EVENT_SCHEMA_VERSION: &str = "singulari.agent_context_event.v1";
pub const AGENT_CONTEXT_EVENTS_FILENAME: &str = "agent_context_events.jsonl";
pub const AGENT_CONTEXT_PROJECTION_SCHEMA_VERSION: &str = "singulari.agent_context_projection.v1";
pub const AGENT_CONTEXT_PROJECTION_FILENAME: &str = "agent_context_projection.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentEntityUpdate {
    pub entity_id: String,
    pub update_kind: String,
    pub visibility: ContextVisibility,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentRelationshipUpdate {
    pub source_entity_id: String,
    pub target_entity_id: String,
    pub relation_kind: String,
    pub visibility: ContextVisibility,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentWorldLoreUpdate {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub category: String,
    pub visibility: ContextVisibility,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCharacterTextDesignUpdate {
    pub character_id: String,
    pub speech_pattern: String,
    pub gesture_pattern: String,
    pub drift_note: String,
    pub visibility: ContextVisibility,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentHiddenStateDelta {
    pub delta_kind: HiddenDeltaKind,
    pub target_id: String,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextVisibility {
    PlayerVisible,
    HiddenAdjudicationOnly,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HiddenDeltaKind {
    SecretStatus,
    Timer,
    Constraint,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentContextEventPlan {
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub records: Vec<AgentContextEventRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentContextEventRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub event_id: String,
    pub event_kind: ContextEventKind,
    pub visibility: ContextVisibility,
    pub target: String,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentContextProjection {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub rebuilt_at: String,
    pub counts: AgentContextProjectionCounts,
    #[serde(default)]
    pub recent_events: Vec<AgentContextEventRecord>,
    #[serde(default)]
    pub entity_summaries: Vec<AgentContextProjectionItem>,
    #[serde(default)]
    pub relationship_summaries: Vec<AgentContextProjectionItem>,
    #[serde(default)]
    pub world_lore_summaries: Vec<AgentContextProjectionItem>,
    #[serde(default)]
    pub character_text_design_summaries: Vec<AgentContextProjectionItem>,
    #[serde(default)]
    pub hidden_delta_summaries: Vec<AgentContextProjectionItem>,
}

impl Default for AgentContextProjection {
    fn default() -> Self {
        Self {
            schema_version: AGENT_CONTEXT_PROJECTION_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            turn_id: String::new(),
            rebuilt_at: String::new(),
            counts: AgentContextProjectionCounts::default(),
            recent_events: Vec::new(),
            entity_summaries: Vec::new(),
            relationship_summaries: Vec::new(),
            world_lore_summaries: Vec::new(),
            character_text_design_summaries: Vec::new(),
            hidden_delta_summaries: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentContextProjectionCounts {
    pub entity: usize,
    pub relationship: usize,
    pub world_lore: usize,
    pub character_text_design: usize,
    pub hidden_state_delta: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentContextProjectionItem {
    pub target: String,
    pub summary: String,
    pub source_event_id: String,
    pub turn_id: String,
    pub visibility: ContextVisibility,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextEventKind {
    Entity,
    Relationship,
    WorldLore,
    CharacterTextDesign,
    HiddenStateDelta,
}

pub fn prepare_agent_context_event_plan(
    world_id: &str,
    turn_id: &str,
    input: &AgentContextEventInput<'_>,
) -> Result<AgentContextEventPlan> {
    let recorded_at = Utc::now().to_rfc3339();
    let mut records = Vec::new();
    for update in input.entity_updates {
        validate_visible_context_fields(
            "entity_updates",
            &[&update.entity_id, &update.update_kind, &update.summary],
            &update.evidence_refs,
        )?;
        records.push(record(
            world_id,
            turn_id,
            records.len(),
            ContextEventKind::Entity,
            update.visibility,
            format!("{}:{}", update.entity_id, update.update_kind),
            update.summary.trim().to_owned(),
            &update.evidence_refs,
            &recorded_at,
        ));
    }
    for update in input.relationship_updates {
        validate_visible_context_fields(
            "relationship_updates",
            &[
                &update.source_entity_id,
                &update.target_entity_id,
                &update.relation_kind,
                &update.summary,
            ],
            &update.evidence_refs,
        )?;
        records.push(record(
            world_id,
            turn_id,
            records.len(),
            ContextEventKind::Relationship,
            update.visibility,
            format!(
                "{}->{}:{}",
                update.source_entity_id, update.target_entity_id, update.relation_kind
            ),
            update.summary.trim().to_owned(),
            &update.evidence_refs,
            &recorded_at,
        ));
    }
    for update in input.world_lore_updates {
        validate_visible_context_fields(
            "world_lore_updates",
            &[
                &update.subject,
                &update.predicate,
                &update.object,
                &update.category,
                &update.summary,
            ],
            &update.evidence_refs,
        )?;
        records.push(record(
            world_id,
            turn_id,
            records.len(),
            ContextEventKind::WorldLore,
            update.visibility,
            format!(
                "{}:{}:{}",
                update.category, update.subject, update.predicate
            ),
            update.summary.trim().to_owned(),
            &update.evidence_refs,
            &recorded_at,
        ));
    }
    for update in input.character_text_design_updates {
        validate_visible_context_fields(
            "character_text_design_updates",
            &[
                &update.character_id,
                &update.speech_pattern,
                &update.gesture_pattern,
                &update.drift_note,
            ],
            &update.evidence_refs,
        )?;
        records.push(record(
            world_id,
            turn_id,
            records.len(),
            ContextEventKind::CharacterTextDesign,
            update.visibility,
            update.character_id.trim().to_owned(),
            format!(
                "speech={}, gesture={}, drift={}",
                update.speech_pattern.trim(),
                update.gesture_pattern.trim(),
                update.drift_note.trim()
            ),
            &update.evidence_refs,
            &recorded_at,
        ));
    }
    for delta in input.hidden_state_delta {
        validate_hidden_delta(delta)?;
        records.push(record(
            world_id,
            turn_id,
            records.len(),
            ContextEventKind::HiddenStateDelta,
            ContextVisibility::HiddenAdjudicationOnly,
            delta.target_id.trim().to_owned(),
            delta.summary.trim().to_owned(),
            &delta.evidence_refs,
            &recorded_at,
        ));
    }
    Ok(AgentContextEventPlan {
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        records,
    })
}

pub struct AgentContextEventInput<'a> {
    pub entity_updates: &'a [AgentEntityUpdate],
    pub relationship_updates: &'a [AgentRelationshipUpdate],
    pub world_lore_updates: &'a [AgentWorldLoreUpdate],
    pub character_text_design_updates: &'a [AgentCharacterTextDesignUpdate],
    pub hidden_state_delta: &'a [AgentHiddenStateDelta],
}

pub fn append_agent_context_event_plan(
    world_dir: &Path,
    plan: &AgentContextEventPlan,
) -> Result<()> {
    for record in &plan.records {
        append_jsonl(&world_dir.join(AGENT_CONTEXT_EVENTS_FILENAME), record)?;
    }
    Ok(())
}

pub fn rebuild_agent_context_projection(world_dir: &Path) -> Result<AgentContextProjection> {
    let records = load_agent_context_event_records(world_dir)?;
    let projection = build_agent_context_projection(&records);
    write_json(
        &world_dir.join(AGENT_CONTEXT_PROJECTION_FILENAME),
        &projection,
    )?;
    Ok(projection)
}

pub fn load_agent_context_projection(world_dir: &Path) -> Result<AgentContextProjection> {
    let path = world_dir.join(AGENT_CONTEXT_PROJECTION_FILENAME);
    if path.exists() {
        return read_json(&path);
    }
    Ok(build_agent_context_projection(
        &load_agent_context_event_records(world_dir)?,
    ))
}

#[must_use]
pub fn build_agent_context_projection(
    records: &[AgentContextEventRecord],
) -> AgentContextProjection {
    let (world_id, turn_id) = records.last().map_or_else(
        || (String::new(), String::new()),
        |record| (record.world_id.clone(), record.turn_id.clone()),
    );
    let mut projection = AgentContextProjection {
        schema_version: AGENT_CONTEXT_PROJECTION_SCHEMA_VERSION.to_owned(),
        world_id,
        turn_id,
        rebuilt_at: Utc::now().to_rfc3339(),
        counts: AgentContextProjectionCounts::default(),
        recent_events: records.iter().rev().take(12).cloned().collect(),
        entity_summaries: Vec::new(),
        relationship_summaries: Vec::new(),
        world_lore_summaries: Vec::new(),
        character_text_design_summaries: Vec::new(),
        hidden_delta_summaries: Vec::new(),
    };
    for record in records {
        let item = projection_item(record);
        match record.event_kind {
            ContextEventKind::Entity => {
                projection.counts.entity += 1;
                upsert_projection_item(&mut projection.entity_summaries, item);
            }
            ContextEventKind::Relationship => {
                projection.counts.relationship += 1;
                upsert_projection_item(&mut projection.relationship_summaries, item);
            }
            ContextEventKind::WorldLore => {
                projection.counts.world_lore += 1;
                upsert_projection_item(&mut projection.world_lore_summaries, item);
            }
            ContextEventKind::CharacterTextDesign => {
                projection.counts.character_text_design += 1;
                upsert_projection_item(&mut projection.character_text_design_summaries, item);
            }
            ContextEventKind::HiddenStateDelta => {
                projection.counts.hidden_state_delta += 1;
                upsert_projection_item(&mut projection.hidden_delta_summaries, item);
            }
        }
    }
    trim_projection_items(&mut projection.entity_summaries, 16);
    trim_projection_items(&mut projection.relationship_summaries, 16);
    trim_projection_items(&mut projection.world_lore_summaries, 16);
    trim_projection_items(&mut projection.character_text_design_summaries, 16);
    trim_projection_items(&mut projection.hidden_delta_summaries, 8);
    projection
}

fn projection_item(record: &AgentContextEventRecord) -> AgentContextProjectionItem {
    AgentContextProjectionItem {
        target: record.target.clone(),
        summary: record.summary.clone(),
        source_event_id: record.event_id.clone(),
        turn_id: record.turn_id.clone(),
        visibility: record.visibility,
    }
}

fn upsert_projection_item(
    items: &mut Vec<AgentContextProjectionItem>,
    item: AgentContextProjectionItem,
) {
    if let Some(existing) = items
        .iter_mut()
        .find(|existing| existing.target == item.target)
    {
        *existing = item;
        return;
    }
    items.push(item);
}

fn trim_projection_items(items: &mut Vec<AgentContextProjectionItem>, limit: usize) {
    if items.len() > limit {
        let overflow = items.len() - limit;
        items.drain(0..overflow);
    }
}

pub fn load_agent_context_event_records(world_dir: &Path) -> Result<Vec<AgentContextEventRecord>> {
    let path = world_dir.join(AGENT_CONTEXT_EVENTS_FILENAME);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|error| anyhow::anyhow!("failed to read {}: {error}", path.display()))?;
    raw.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str::<AgentContextEventRecord>(line).map_err(|error| {
                anyhow::anyhow!(
                    "failed to parse {} line {} as AgentContextEventRecord: {error}",
                    path.display(),
                    index + 1
                )
            })
        })
        .collect()
}

fn validate_visible_context_fields(
    surface: &str,
    fields: &[&str],
    evidence_refs: &[String],
) -> Result<()> {
    if fields.iter().any(|field| field.trim().is_empty()) {
        bail!("{surface} contains an empty required field");
    }
    if evidence_refs.is_empty()
        || evidence_refs
            .iter()
            .any(|reference| reference.trim().is_empty())
    {
        bail!("{surface} evidence_refs must contain non-empty visible refs");
    }
    Ok(())
}

fn validate_hidden_delta(delta: &AgentHiddenStateDelta) -> Result<()> {
    if delta.target_id.trim().is_empty() || delta.summary.trim().is_empty() {
        bail!("hidden_state_delta contains an empty required field");
    }
    if delta.evidence_refs.is_empty()
        || delta
            .evidence_refs
            .iter()
            .any(|reference| reference.trim().is_empty())
    {
        bail!("hidden_state_delta evidence_refs must contain non-empty adjudication refs");
    }
    Ok(())
}

fn record(
    world_id: &str,
    turn_id: &str,
    index: usize,
    event_kind: ContextEventKind,
    visibility: ContextVisibility,
    target: String,
    summary: String,
    evidence_refs: &[String],
    recorded_at: &str,
) -> AgentContextEventRecord {
    AgentContextEventRecord {
        schema_version: AGENT_CONTEXT_EVENT_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        event_id: format!("agent_context_event:{turn_id}:{index:02}"),
        event_kind,
        visibility,
        target,
        summary,
        evidence_refs: evidence_refs
            .iter()
            .map(|reference| reference.trim().to_owned())
            .collect(),
        recorded_at: recorded_at.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepares_typed_context_events() -> Result<()> {
        let plan = prepare_agent_context_event_plan(
            "stw_context",
            "turn_0001",
            &AgentContextEventInput {
                entity_updates: &[AgentEntityUpdate {
                    entity_id: "char:guard".to_owned(),
                    update_kind: "seen_action".to_owned(),
                    visibility: ContextVisibility::PlayerVisible,
                    summary: "guard saw the protagonist hesitate".to_owned(),
                    evidence_refs: vec!["visible_scene.text_blocks[0]".to_owned()],
                }],
                relationship_updates: &[],
                world_lore_updates: &[],
                character_text_design_updates: &[],
                hidden_state_delta: &[],
            },
        )?;

        assert_eq!(plan.records.len(), 1);
        assert_eq!(plan.records[0].event_kind, ContextEventKind::Entity);
        Ok(())
    }

    #[test]
    fn rejects_context_event_without_evidence() {
        let Err(error) = prepare_agent_context_event_plan(
            "stw_context",
            "turn_0001",
            &AgentContextEventInput {
                entity_updates: &[AgentEntityUpdate {
                    entity_id: "char:guard".to_owned(),
                    update_kind: "seen_action".to_owned(),
                    visibility: ContextVisibility::PlayerVisible,
                    summary: "guard saw the protagonist hesitate".to_owned(),
                    evidence_refs: Vec::new(),
                }],
                relationship_updates: &[],
                world_lore_updates: &[],
                character_text_design_updates: &[],
                hidden_state_delta: &[],
            },
        ) else {
            panic!("missing evidence refs must reject context event");
        };

        assert!(error.to_string().contains("entity_updates evidence_refs"));
    }

    #[test]
    fn projection_keeps_latest_item_per_target() {
        let records = vec![
            AgentContextEventRecord {
                schema_version: AGENT_CONTEXT_EVENT_SCHEMA_VERSION.to_owned(),
                world_id: "stw_context".to_owned(),
                turn_id: "turn_0001".to_owned(),
                event_id: "evt_1".to_owned(),
                event_kind: ContextEventKind::Relationship,
                visibility: ContextVisibility::PlayerVisible,
                target: "char:a->char:b:trust".to_owned(),
                summary: "first stance".to_owned(),
                evidence_refs: vec!["visible_scene.text_blocks[0]".to_owned()],
                recorded_at: "2026-04-29T00:00:00Z".to_owned(),
            },
            AgentContextEventRecord {
                schema_version: AGENT_CONTEXT_EVENT_SCHEMA_VERSION.to_owned(),
                world_id: "stw_context".to_owned(),
                turn_id: "turn_0002".to_owned(),
                event_id: "evt_2".to_owned(),
                event_kind: ContextEventKind::Relationship,
                visibility: ContextVisibility::PlayerVisible,
                target: "char:a->char:b:trust".to_owned(),
                summary: "latest stance".to_owned(),
                evidence_refs: vec!["visible_scene.text_blocks[0]".to_owned()],
                recorded_at: "2026-04-29T00:00:01Z".to_owned(),
            },
        ];

        let projection = build_agent_context_projection(&records);

        assert_eq!(projection.counts.relationship, 2);
        assert_eq!(projection.relationship_summaries.len(), 1);
        assert_eq!(
            projection.relationship_summaries[0].summary,
            "latest stance"
        );
    }
}
