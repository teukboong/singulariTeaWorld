use crate::response_context::{AgentContextProjection, AgentWorldLoreUpdate, ContextVisibility};
use crate::store::{append_jsonl, read_json, write_json};
use crate::world_db::WorldFactRow;
use anyhow::{Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const WORLD_LORE_PACKET_SCHEMA_VERSION: &str = "singulari.world_lore_packet.v1";
pub const WORLD_LORE_ENTRY_SCHEMA_VERSION: &str = "singulari.world_lore_entry.v1";
pub const WORLD_LORE_UPDATE_SCHEMA_VERSION: &str = "singulari.world_lore_update.v1";
pub const WORLD_LORE_FILENAME: &str = "world_lore.json";
pub const WORLD_LORE_UPDATES_FILENAME: &str = "world_lore_updates.jsonl";

const WORLD_LORE_ENTRY_BUDGET: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldLorePacket {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub entries: Vec<WorldLoreEntry>,
    pub compiler_policy: WorldLorePolicy,
}

impl Default for WorldLorePacket {
    fn default() -> Self {
        Self {
            schema_version: WORLD_LORE_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            turn_id: String::new(),
            entries: Vec::new(),
            compiler_policy: WorldLorePolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldLoreEntry {
    pub schema_version: String,
    pub lore_id: String,
    pub domain: WorldLoreDomain,
    pub name: String,
    pub summary: String,
    pub visibility: String,
    pub confidence: String,
    pub authority: String,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default)]
    pub mechanical_axis: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldLoreUpdatePlan {
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub records: Vec<WorldLoreUpdateRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldLoreUpdateRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub update_id: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub category: String,
    pub visibility: ContextVisibility,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldLorePolicy {
    pub source: String,
    pub entry_budget: usize,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for WorldLorePolicy {
    fn default() -> Self {
        Self {
            source: "compiled_from_world_facts_v0".to_owned(),
            entry_budget: WORLD_LORE_ENTRY_BUDGET,
            use_rules: vec![
                "Use active_world_lore as player-visible world constraints, not as license to invent genre lore.".to_owned(),
                "Style contracts and examples cannot create lore entries.".to_owned(),
                "If a scene establishes a durable custom, resource, institution, route, or social rule, later phases must record it through structured world_lore_updates.".to_owned(),
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorldLoreDomain {
    Geography,
    Settlements,
    SocialOrder,
    Economy,
    FaithAndMyth,
    TechnologyLevel,
    DangerModel,
    Customs,
    LanguageRegister,
    KnownUnknowns,
}

#[must_use]
pub fn compile_world_lore_packet(
    world_id: &str,
    turn_id: &str,
    facts: &[WorldFactRow],
) -> WorldLorePacket {
    let entries = facts
        .iter()
        .take(WORLD_LORE_ENTRY_BUDGET)
        .map(world_fact_lore_entry)
        .collect();
    WorldLorePacket {
        schema_version: WORLD_LORE_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        entries,
        compiler_policy: WorldLorePolicy::default(),
    }
}

#[must_use]
pub fn compile_world_lore_from_projection(
    world_id: &str,
    turn_id: &str,
    projection: &AgentContextProjection,
    base_facts: &[WorldFactRow],
) -> WorldLorePacket {
    if projection.world_lore_summaries.is_empty() {
        return compile_world_lore_packet(world_id, turn_id, base_facts);
    }
    let entries = projection
        .world_lore_summaries
        .iter()
        .filter(|item| item.visibility == ContextVisibility::PlayerVisible)
        .take(WORLD_LORE_ENTRY_BUDGET)
        .map(|item| {
            let (category, name) = parse_lore_projection_target(item.target.as_str());
            WorldLoreEntry {
                schema_version: WORLD_LORE_ENTRY_SCHEMA_VERSION.to_owned(),
                lore_id: format!("lore:agent_context:{}", item.source_event_id),
                domain: domain_for_category(category.as_str()),
                name,
                summary: item.summary.clone(),
                visibility: "player_visible".to_owned(),
                confidence: "confirmed".to_owned(),
                authority: "agent_context_projection".to_owned(),
                source_refs: vec![format!("agent_context_event:{}", item.source_event_id)],
                mechanical_axis: mechanical_axis_for_category(category.as_str()),
            }
        })
        .collect();
    WorldLorePacket {
        schema_version: WORLD_LORE_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        entries,
        compiler_policy: WorldLorePolicy {
            source: "compiled_from_agent_context_projection_v1".to_owned(),
            ..WorldLorePolicy::default()
        },
    }
}

/// Validate agent-authored lore updates before the turn is advanced.
///
/// # Errors
///
/// Returns an error when an update is missing required fields or visible
/// evidence references.
pub fn prepare_world_lore_update_plan(
    world_id: &str,
    turn_id: &str,
    updates: &[AgentWorldLoreUpdate],
) -> Result<WorldLoreUpdatePlan> {
    let recorded_at = Utc::now().to_rfc3339();
    let mut records = Vec::new();
    for update in updates {
        validate_world_lore_update(update)?;
        records.push(WorldLoreUpdateRecord {
            schema_version: WORLD_LORE_UPDATE_SCHEMA_VERSION.to_owned(),
            world_id: world_id.to_owned(),
            turn_id: turn_id.to_owned(),
            update_id: format!("world_lore_update:{turn_id}:{:02}", records.len()),
            subject: update.subject.trim().to_owned(),
            predicate: update.predicate.trim().to_owned(),
            object: update.object.trim().to_owned(),
            category: update.category.trim().to_owned(),
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
    Ok(WorldLoreUpdatePlan {
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        records,
    })
}

/// Append prevalidated world-lore update records to the durable event log.
///
/// # Errors
///
/// Returns an error when the update log cannot be written.
pub fn append_world_lore_update_plan(world_dir: &Path, plan: &WorldLoreUpdatePlan) -> Result<()> {
    for record in &plan.records {
        append_jsonl(&world_dir.join(WORLD_LORE_UPDATES_FILENAME), record)?;
    }
    Ok(())
}

/// Rebuild the materialized world-lore packet from the durable update log.
///
/// # Errors
///
/// Returns an error when update records cannot be read or the materialized
/// packet cannot be written.
pub fn rebuild_world_lore(
    world_dir: &Path,
    base_packet: &WorldLorePacket,
) -> Result<WorldLorePacket> {
    let records = load_world_lore_update_records(world_dir)?;
    let packet = build_world_lore_from_updates(base_packet, &records);
    write_json(&world_dir.join(WORLD_LORE_FILENAME), &packet)?;
    Ok(packet)
}

/// Load the materialized world-lore packet, or the supplied base packet for
/// legacy worlds that do not have the dedicated file yet.
///
/// # Errors
///
/// Returns an error when an existing materialized packet cannot be parsed.
pub fn load_world_lore_state(
    world_dir: &Path,
    base_packet: WorldLorePacket,
) -> Result<WorldLorePacket> {
    let path = world_dir.join(WORLD_LORE_FILENAME);
    if path.is_file() {
        return read_json(&path);
    }
    Ok(base_packet)
}

#[must_use]
pub fn build_world_lore_from_updates(
    base_packet: &WorldLorePacket,
    records: &[WorldLoreUpdateRecord],
) -> WorldLorePacket {
    let mut entries = base_packet.entries.clone();
    for record in records
        .iter()
        .filter(|record| record.visibility == ContextVisibility::PlayerVisible)
    {
        let entry = world_lore_entry_from_update(record);
        if let Some(existing) = entries
            .iter_mut()
            .find(|existing| existing.lore_id == entry.lore_id)
        {
            *existing = entry;
        } else {
            entries.push(entry);
        }
    }
    if entries.len() > WORLD_LORE_ENTRY_BUDGET {
        let overflow = entries.len() - WORLD_LORE_ENTRY_BUDGET;
        entries.drain(0..overflow);
    }
    WorldLorePacket {
        schema_version: WORLD_LORE_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: base_packet.world_id.clone(),
        turn_id: records.last().map_or_else(
            || base_packet.turn_id.clone(),
            |record| record.turn_id.clone(),
        ),
        entries,
        compiler_policy: WorldLorePolicy {
            source: "materialized_from_world_lore_updates_v1".to_owned(),
            ..WorldLorePolicy::default()
        },
    }
}

/// Load world-lore update records from the durable event log.
///
/// # Errors
///
/// Returns an error when the update log cannot be read or contains malformed
/// JSON lines.
pub fn load_world_lore_update_records(world_dir: &Path) -> Result<Vec<WorldLoreUpdateRecord>> {
    let path = world_dir.join(WORLD_LORE_UPDATES_FILENAME);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|error| anyhow::anyhow!("failed to read {}: {error}", path.display()))?;
    raw.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str::<WorldLoreUpdateRecord>(line).map_err(|error| {
                anyhow::anyhow!(
                    "failed to parse {} line {} as WorldLoreUpdateRecord: {error}",
                    path.display(),
                    index + 1
                )
            })
        })
        .collect()
}

fn parse_lore_projection_target(target: &str) -> (String, String) {
    let mut parts = target.split(':');
    let category = parts.next().unwrap_or("world").to_owned();
    let subject = parts.next().unwrap_or(target).to_owned();
    (category, subject)
}

fn world_fact_lore_entry(fact: &WorldFactRow) -> WorldLoreEntry {
    WorldLoreEntry {
        schema_version: WORLD_LORE_ENTRY_SCHEMA_VERSION.to_owned(),
        lore_id: format!("lore:world_fact:{}", fact.fact_id),
        domain: domain_for_category(fact.category.as_str()),
        name: fact.subject.clone(),
        summary: format!("{} {} {}", fact.subject, fact.predicate, fact.object),
        visibility: "player_visible".to_owned(),
        confidence: "confirmed".to_owned(),
        authority: "system_projection".to_owned(),
        source_refs: vec![format!("world_facts:{}", fact.fact_id)],
        mechanical_axis: mechanical_axis_for_category(fact.category.as_str()),
    }
}

fn world_lore_entry_from_update(record: &WorldLoreUpdateRecord) -> WorldLoreEntry {
    WorldLoreEntry {
        schema_version: WORLD_LORE_ENTRY_SCHEMA_VERSION.to_owned(),
        lore_id: format!(
            "lore:{}:{}:{}",
            record.category, record.subject, record.predicate
        ),
        domain: domain_for_category(record.category.as_str()),
        name: record.subject.clone(),
        summary: record.summary.clone(),
        visibility: "player_visible".to_owned(),
        confidence: "confirmed".to_owned(),
        authority: "world_lore_updates".to_owned(),
        source_refs: vec![format!("world_lore_update:{}", record.update_id)],
        mechanical_axis: mechanical_axis_for_category(record.category.as_str()),
    }
}

fn validate_world_lore_update(update: &AgentWorldLoreUpdate) -> Result<()> {
    let fields = [
        update.subject.as_str(),
        update.predicate.as_str(),
        update.object.as_str(),
        update.category.as_str(),
        update.summary.as_str(),
    ];
    if fields.iter().any(|field| field.trim().is_empty()) {
        bail!("world_lore_updates contains an empty required field");
    }
    if update.evidence_refs.is_empty()
        || update
            .evidence_refs
            .iter()
            .any(|reference| reference.trim().is_empty())
    {
        bail!("world_lore_updates evidence_refs must contain non-empty visible refs");
    }
    Ok(())
}

fn domain_for_category(category: &str) -> WorldLoreDomain {
    match category {
        "law" | "runtime_contract" => WorldLoreDomain::SocialOrder,
        "language" => WorldLoreDomain::LanguageRegister,
        _ => WorldLoreDomain::KnownUnknowns,
    }
}

fn mechanical_axis_for_category(category: &str) -> Vec<String> {
    match category {
        "law" | "runtime_contract" => vec!["knowledge".to_owned(), "moral_cost".to_owned()],
        "language" => vec!["social_permission".to_owned()],
        _ => vec!["knowledge".to_owned()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_world_facts_as_lore_entries() {
        let facts = vec![WorldFactRow {
            fact_id: "fact:law:death".to_owned(),
            category: "law".to_owned(),
            subject: "death".to_owned(),
            predicate: "is".to_owned(),
            object: "final".to_owned(),
        }];

        let packet = compile_world_lore_packet("stw_lore", "turn_0001", &facts);

        assert_eq!(packet.entries.len(), 1);
        assert_eq!(packet.entries[0].domain, WorldLoreDomain::SocialOrder);
        assert_eq!(
            packet.entries[0].mechanical_axis,
            vec!["knowledge", "moral_cost"]
        );
    }

    #[test]
    fn projection_overrides_world_facts() {
        let projection = AgentContextProjection {
            world_id: "stw_lore".to_owned(),
            turn_id: "turn_0002".to_owned(),
            world_lore_summaries: vec![crate::response_context::AgentContextProjectionItem {
                target: "customs:gate_tax:requires".to_owned(),
                summary: "gate tax requires a stamped token".to_owned(),
                source_event_id: "ctx_1".to_owned(),
                turn_id: "turn_0002".to_owned(),
                visibility: ContextVisibility::PlayerVisible,
            }],
            ..AgentContextProjection::default()
        };

        let packet = compile_world_lore_from_projection("stw_lore", "turn_0002", &projection, &[]);

        assert_eq!(
            packet.compiler_policy.source,
            "compiled_from_agent_context_projection_v1"
        );
        assert_eq!(packet.entries[0].name, "gate_tax");
        assert_eq!(packet.entries[0].authority, "agent_context_projection");
    }

    #[test]
    fn materializes_world_lore_updates_over_base_packet() -> Result<()> {
        let base_packet = WorldLorePacket {
            world_id: "stw_lore".to_owned(),
            turn_id: "turn_0001".to_owned(),
            ..WorldLorePacket::default()
        };
        let plan = prepare_world_lore_update_plan(
            "stw_lore",
            "turn_0002",
            &[AgentWorldLoreUpdate {
                subject: "gate tax".to_owned(),
                predicate: "requires".to_owned(),
                object: "stamped token".to_owned(),
                category: "customs".to_owned(),
                visibility: ContextVisibility::PlayerVisible,
                summary: "gate tax requires a stamped token".to_owned(),
                evidence_refs: vec!["visible_scene.text_blocks[0]".to_owned()],
            }],
        )?;

        let packet = build_world_lore_from_updates(&base_packet, &plan.records);

        assert_eq!(packet.entries.len(), 1);
        assert_eq!(packet.entries[0].authority, "world_lore_updates");
        assert_eq!(
            packet.compiler_policy.source,
            "materialized_from_world_lore_updates_v1"
        );
        Ok(())
    }
}
