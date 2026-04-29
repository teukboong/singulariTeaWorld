use crate::agent_bridge::PendingAgentTurn;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const TURN_CONTEXT_PACKET_SCHEMA_VERSION: &str = "singulari.turn_context_packet.v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnContextPacket {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub source_revival: Value,
    pub assembly_policy: TurnContextAssemblyPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnContextAssemblyPolicy {
    pub source: String,
    #[serde(default)]
    pub visible_sections: Vec<String>,
    #[serde(default)]
    pub adjudication_only_sections: Vec<String>,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for TurnContextAssemblyPolicy {
    fn default() -> Self {
        Self {
            source: "turn_context_assembly_v0".to_owned(),
            visible_sections: vec![
                "memory_revival.recent_scene_window".to_owned(),
                "memory_revival.known_facts".to_owned(),
                "memory_revival.active_scene_pressure.visible_active".to_owned(),
                "memory_revival.active_plot_threads.active_visible".to_owned(),
                "memory_revival.active_body_resource_state".to_owned(),
                "memory_revival.active_location_graph".to_owned(),
                "memory_revival.active_character_text_design".to_owned(),
                "memory_revival.active_memory_revival.active_relationship_graph".to_owned(),
                "memory_revival.active_memory_revival.active_world_lore".to_owned(),
            ],
            adjudication_only_sections: vec![
                "private_adjudication_context".to_owned(),
                "memory_revival.active_scene_pressure.hidden_adjudication_only".to_owned(),
            ],
            use_rules: vec![
                "Source revival remains the source packet; turn context assembly only labels visibility and prompt-use boundaries.".to_owned(),
                "Visible sections may shape VN prose and choices.".to_owned(),
                "Adjudication-only sections may shape outcomes but must not leak into visible text, choices, image prompts, or player projections.".to_owned(),
            ],
        }
    }
}

#[must_use]
pub fn assemble_turn_context_packet(
    pending: &PendingAgentTurn,
    source_revival: Value,
) -> TurnContextPacket {
    TurnContextPacket {
        schema_version: TURN_CONTEXT_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: pending.world_id.clone(),
        turn_id: pending.turn_id.clone(),
        source_revival,
        assembly_policy: TurnContextAssemblyPolicy::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_bridge::{
        AgentOutputContract, AgentPrivateAdjudicationContext, AgentVisibleContext,
    };

    #[test]
    fn turn_context_marks_hidden_pressure_as_adjudication_only() {
        let pending = PendingAgentTurn {
            schema_version: "singulari.agent_pending_turn.v1".to_owned(),
            world_id: "stw_context".to_owned(),
            turn_id: "turn_0001".to_owned(),
            status: "pending".to_owned(),
            player_input: "1".to_owned(),
            selected_choice: None,
            visible_context: AgentVisibleContext {
                location: "place:opening".to_owned(),
                recent_scene: Vec::new(),
                known_facts: Vec::new(),
                voice_anchors: Vec::new(),
                extra_memory: Default::default(),
                active_scene_pressure: Default::default(),
                active_plot_threads: Default::default(),
                active_body_resource_state: Default::default(),
                active_location_graph: Default::default(),
                active_character_text_design: Default::default(),
            },
            private_adjudication_context: AgentPrivateAdjudicationContext {
                hidden_timers: Vec::new(),
                unrevealed_constraints: Vec::new(),
                plausibility_gates: Vec::new(),
            },
            output_contract: AgentOutputContract {
                language: "ko".to_owned(),
                must_return_json: true,
                hidden_truth_must_not_appear_in_visible_text: true,
                narrative_level: 1,
                narrative_budget: crate::agent_bridge::narrative_budget_for_level(Some(1)),
            },
            pending_ref: "pending".to_owned(),
            created_at: "2026-04-29T00:00:00Z".to_owned(),
        };

        let packet = assemble_turn_context_packet(&pending, serde_json::json!({"ok": true}));

        assert!(
            packet.assembly_policy.adjudication_only_sections.contains(
                &"memory_revival.active_scene_pressure.hidden_adjudication_only".to_owned()
            )
        );
    }
}
