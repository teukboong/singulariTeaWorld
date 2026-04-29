use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const BELIEF_GRAPH_PACKET_SCHEMA_VERSION: &str = "singulari.belief_graph_packet.v1";
pub const BELIEF_NODE_SCHEMA_VERSION: &str = "singulari.belief_node.v1";

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
