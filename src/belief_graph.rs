#![allow(clippy::missing_errors_doc)]

use crate::agent_bridge::AgentTurnResponse;
use crate::store::{append_jsonl, read_json, write_json};
use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

pub const BELIEF_GRAPH_PACKET_SCHEMA_VERSION: &str = "singulari.belief_graph_packet.v1";
pub const BELIEF_NODE_SCHEMA_VERSION: &str = "singulari.belief_node.v1";
pub const BELIEF_EVENT_SCHEMA_VERSION: &str = "singulari.belief_event.v1";
pub const BELIEF_GRAPH_FILENAME: &str = "belief_graph.json";
pub const BELIEF_EVENTS_FILENAME: &str = "belief_events.jsonl";

const PERSISTED_BELIEF_BUDGET: usize = 16;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BeliefGraphPacket {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub protagonist_visible_beliefs: Vec<BeliefNode>,
    #[serde(default)]
    pub narrator_knowledge_limits: Vec<String>,
    pub compiler_policy: BeliefGraphPolicy,
}

impl Default for BeliefGraphPacket {
    fn default() -> Self {
        Self {
            schema_version: BELIEF_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            turn_id: String::new(),
            protagonist_visible_beliefs: Vec::new(),
            narrator_knowledge_limits: Vec::new(),
            compiler_policy: BeliefGraphPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BeliefNode {
    pub schema_version: String,
    pub belief_id: String,
    pub holder: BeliefHolder,
    pub confidence: BeliefConfidence,
    pub statement: String,
    #[serde(default)]
    pub source_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BeliefHolder {
    Protagonist,
    PlayerVisibleNarrator,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BeliefConfidence {
    Observed,
    Inferred,
    Reported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BeliefGraphPolicy {
    pub source: String,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BeliefEventPlan {
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub records: Vec<BeliefEventRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BeliefEventRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub event_id: String,
    pub holder: BeliefHolder,
    pub confidence: BeliefConfidence,
    pub statement: String,
    #[serde(default)]
    pub source_refs: Vec<String>,
    pub recorded_at: String,
}

impl Default for BeliefGraphPolicy {
    fn default() -> Self {
        Self {
            source: "compiled_from_visible_facts_and_selected_memory_v1".to_owned(),
            use_rules: vec![
                "Narration may state observed beliefs directly, but inferred or reported beliefs must keep uncertainty in prose.".to_owned(),
                "Do not reveal hidden causes, future outcomes, private motives, or exact identities unless represented by a belief node.".to_owned(),
                "A missing belief node means unknown, not permission to invent an explanation.".to_owned(),
            ],
        }
    }
}

#[must_use]
pub fn compile_belief_graph_packet(
    world_id: &str,
    turn_id: &str,
    known_facts: &Value,
    selected_memory_items: &Value,
) -> BeliefGraphPacket {
    let mut beliefs = Vec::new();
    collect_array_beliefs(
        &mut beliefs,
        known_facts,
        BeliefConfidence::Observed,
        "known_facts",
    );
    collect_selected_memory_beliefs(&mut beliefs, selected_memory_items);

    BeliefGraphPacket {
        schema_version: BELIEF_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        protagonist_visible_beliefs: beliefs,
        narrator_knowledge_limits: vec![
            "주인공이 직접 보거나 들은 것, 또는 selected_memory_items로 제공된 사실만 확정 서술한다."
                .to_owned(),
            "원인, 정체, 배후, 과거사, 장면 밖 세계 규칙은 belief node가 없으면 유보한다.".to_owned(),
            "불확실한 사실은 몸의 반응, 관찰 가능한 단서, 타인의 말투로만 드러낸다.".to_owned(),
        ],
        compiler_policy: BeliefGraphPolicy::default(),
    }
}

#[must_use]
pub fn prepare_belief_event_plan(
    world_id: &str,
    turn_id: &str,
    response: &AgentTurnResponse,
) -> BeliefEventPlan {
    let recorded_at = Utc::now().to_rfc3339();
    let mut records = Vec::new();
    if let Some(canon_event) = &response.canon_event
        && canon_event.visibility == "player_visible"
    {
        records.push(belief_event(BeliefEventSeed {
            world_id,
            turn_id,
            index: records.len(),
            holder: BeliefHolder::PlayerVisibleNarrator,
            confidence: BeliefConfidence::Observed,
            statement: canon_event.summary.trim(),
            source_refs: vec![format!("canon_event:{turn_id}")],
            recorded_at: recorded_at.as_str(),
        }));
    }
    for constraint in response
        .adjudication
        .iter()
        .flat_map(|adjudication| adjudication.visible_constraints.iter())
    {
        records.push(belief_event(BeliefEventSeed {
            world_id,
            turn_id,
            index: records.len(),
            holder: BeliefHolder::Protagonist,
            confidence: BeliefConfidence::Inferred,
            statement: constraint.trim(),
            source_refs: vec![format!("adjudication:{turn_id}")],
            recorded_at: recorded_at.as_str(),
        }));
    }
    BeliefEventPlan {
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        records,
    }
}

pub fn append_belief_event_plan(world_dir: &Path, plan: &BeliefEventPlan) -> Result<()> {
    for record in &plan.records {
        append_jsonl(&world_dir.join(BELIEF_EVENTS_FILENAME), record)?;
    }
    Ok(())
}

pub fn rebuild_belief_graph(
    world_dir: &Path,
    base_packet: &BeliefGraphPacket,
) -> Result<BeliefGraphPacket> {
    let mut protagonist_visible_beliefs = base_packet.protagonist_visible_beliefs.clone();
    protagonist_visible_beliefs.extend(
        load_belief_event_records(world_dir)?
            .into_iter()
            .rev()
            .take(PERSISTED_BELIEF_BUDGET)
            .map(belief_node_from_event),
    );
    protagonist_visible_beliefs.truncate(PERSISTED_BELIEF_BUDGET);
    let packet = BeliefGraphPacket {
        schema_version: BELIEF_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: base_packet.world_id.clone(),
        turn_id: base_packet.turn_id.clone(),
        protagonist_visible_beliefs,
        narrator_knowledge_limits: base_packet.narrator_knowledge_limits.clone(),
        compiler_policy: BeliefGraphPolicy {
            source: "materialized_from_belief_events_v1".to_owned(),
            ..BeliefGraphPolicy::default()
        },
    };
    write_json(&world_dir.join(BELIEF_GRAPH_FILENAME), &packet)?;
    Ok(packet)
}

pub fn load_belief_graph_state(
    world_dir: &Path,
    base_packet: BeliefGraphPacket,
) -> Result<BeliefGraphPacket> {
    let path = world_dir.join(BELIEF_GRAPH_FILENAME);
    if path.is_file() {
        return read_json(&path);
    }
    Ok(base_packet)
}

fn load_belief_event_records(world_dir: &Path) -> Result<Vec<BeliefEventRecord>> {
    let path = world_dir.join(BELIEF_EVENTS_FILENAME);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)?;
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str::<BeliefEventRecord>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

struct BeliefEventSeed<'a> {
    world_id: &'a str,
    turn_id: &'a str,
    index: usize,
    holder: BeliefHolder,
    confidence: BeliefConfidence,
    statement: &'a str,
    source_refs: Vec<String>,
    recorded_at: &'a str,
}

fn belief_event(seed: BeliefEventSeed<'_>) -> BeliefEventRecord {
    BeliefEventRecord {
        schema_version: BELIEF_EVENT_SCHEMA_VERSION.to_owned(),
        world_id: seed.world_id.to_owned(),
        turn_id: seed.turn_id.to_owned(),
        event_id: format!("belief_event:{}:{:02}", seed.turn_id, seed.index),
        holder: seed.holder,
        confidence: seed.confidence,
        statement: seed.statement.to_owned(),
        source_refs: seed.source_refs,
        recorded_at: seed.recorded_at.to_owned(),
    }
}

fn belief_node_from_event(record: BeliefEventRecord) -> BeliefNode {
    BeliefNode {
        schema_version: BELIEF_NODE_SCHEMA_VERSION.to_owned(),
        belief_id: format!("belief:event:{}", record.event_id),
        holder: record.holder,
        confidence: record.confidence,
        statement: record.statement,
        source_refs: record.source_refs,
    }
}

fn collect_array_beliefs(
    beliefs: &mut Vec<BeliefNode>,
    value: &Value,
    confidence: BeliefConfidence,
    source_label: &str,
) {
    let Value::Array(items) = value else {
        return;
    };
    for (index, item) in items.iter().enumerate() {
        let Some(statement) = visible_statement(item) else {
            continue;
        };
        beliefs.push(BeliefNode {
            schema_version: BELIEF_NODE_SCHEMA_VERSION.to_owned(),
            belief_id: format!("belief:{source_label}:{index:02}"),
            holder: BeliefHolder::Protagonist,
            confidence,
            statement,
            source_refs: vec![format!("{source_label}[{index}]")],
        });
    }
}

fn collect_selected_memory_beliefs(beliefs: &mut Vec<BeliefNode>, selected_memory_items: &Value) {
    let Value::Array(items) = selected_memory_items else {
        return;
    };
    for (index, item) in items.iter().enumerate() {
        let Some(statement) = visible_statement(
            item.pointer("/payload/visible_summary")
                .unwrap_or(item.pointer("/payload/summary").unwrap_or(item)),
        ) else {
            continue;
        };
        let source_id = item
            .pointer("/source_id")
            .and_then(Value::as_str)
            .map_or_else(
                || format!("selected_memory_items[{index}]"),
                ToOwned::to_owned,
            );
        beliefs.push(BeliefNode {
            schema_version: BELIEF_NODE_SCHEMA_VERSION.to_owned(),
            belief_id: format!("belief:selected_memory:{index:02}"),
            holder: BeliefHolder::PlayerVisibleNarrator,
            confidence: BeliefConfidence::Reported,
            statement,
            source_refs: vec![source_id],
        });
    }
}

fn visible_statement(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => non_empty(text),
        Value::Object(object) => object
            .get("summary")
            .or_else(|| object.get("visible_summary"))
            .or_else(|| object.get("statement"))
            .and_then(Value::as_str)
            .and_then(non_empty),
        _ => None,
    }
}

fn non_empty(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_visible_beliefs_without_hidden_explanation() {
        let graph = compile_belief_graph_packet(
            "stw_belief",
            "turn_0004",
            &serde_json::json!(["문이 안쪽에서 막혀 있다"]),
            &serde_json::json!([{
                "source_id": "lore:visible:door",
                "payload": {"visible_summary": "낡은 문은 자주 습기를 먹는다"}
            }]),
        );

        assert_eq!(graph.protagonist_visible_beliefs.len(), 2);
        assert_eq!(
            graph.protagonist_visible_beliefs[0].confidence,
            BeliefConfidence::Observed
        );
        assert_eq!(
            graph.protagonist_visible_beliefs[1].confidence,
            BeliefConfidence::Reported
        );
        assert!(
            graph
                .narrator_knowledge_limits
                .iter()
                .any(|rule| rule.contains("belief node가 없으면 유보"))
        );
    }
}
