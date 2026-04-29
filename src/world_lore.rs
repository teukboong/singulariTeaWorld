use crate::response_context::{AgentContextProjection, ContextVisibility};
use crate::world_db::WorldFactRow;
use serde::{Deserialize, Serialize};

pub const WORLD_LORE_PACKET_SCHEMA_VERSION: &str = "singulari.world_lore_packet.v1";
pub const WORLD_LORE_ENTRY_SCHEMA_VERSION: &str = "singulari.world_lore_entry.v1";

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
    fallback_facts: &[WorldFactRow],
) -> WorldLorePacket {
    if projection.world_lore_summaries.is_empty() {
        return compile_world_lore_packet(world_id, turn_id, fallback_facts);
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

fn domain_for_category(category: &str) -> WorldLoreDomain {
    match category {
        "premise" | "world" => WorldLoreDomain::KnownUnknowns,
        "law" | "runtime_contract" => WorldLoreDomain::SocialOrder,
        "anchor" => WorldLoreDomain::KnownUnknowns,
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
}
