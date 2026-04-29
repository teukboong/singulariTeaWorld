use crate::affordance_graph::compile_affordance_graph_packet;
use crate::agent_bridge::AgentOutputContract;
use crate::belief_graph::compile_belief_graph_packet;
use crate::body_resource::BodyResourcePacket;
use crate::location_graph::LocationGraphPacket;
use crate::narrative_style_state::compile_narrative_style_state;
use crate::scene_pressure::ScenePressurePacket;
use crate::turn_context::TurnContextPacket;
use crate::world_process_clock::compile_world_process_clock_packet;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROMPT_CONTEXT_PACKET_SCHEMA_VERSION: &str = "singulari.prompt_context_packet.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PromptContextPacket {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub current_turn: Value,
    pub opening_randomizer: Value,
    pub output_contract: Value,
    pub visible_context: PromptVisibleContext,
    pub adjudication_context: PromptAdjudicationContext,
    pub source_of_truth_policy: Value,
    pub prompt_policy: PromptContextPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PromptVisibleContext {
    pub recent_scene_window: Value,
    pub known_facts: Value,
    pub active_scene_pressure: Value,
    pub active_plot_threads: Value,
    pub active_body_resource_state: Value,
    pub active_location_graph: Value,
    pub affordance_graph: Value,
    pub belief_graph: Value,
    pub world_process_clock: Value,
    pub narrative_style_state: Value,
    pub active_character_text_design: Value,
    pub selected_memory_items: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PromptAdjudicationContext {
    pub private_adjudication_context: Value,
    pub hidden_scene_pressure: Value,
    pub hidden_world_process_clock: Value,
    pub selected_adjudication_items: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptContextPolicy {
    pub source: String,
    #[serde(default)]
    pub omitted_debug_sections: Vec<String>,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for PromptContextPolicy {
    fn default() -> Self {
        Self {
            source: "prompt_context_assembly_v1".to_owned(),
            omitted_debug_sections: vec![
                "source_revival.memory_revival.resume_pack".to_owned(),
                "source_revival.memory_revival.active_memory_revival.player_visible_archive_view"
                    .to_owned(),
                "source_revival.memory_revival.active_memory_revival.query_recall".to_owned(),
                "source_revival.memory_revival.active_memory_revival.recent_entity_updates"
                    .to_owned(),
                "source_revival.memory_revival.active_memory_revival.recent_relationship_updates"
                    .to_owned(),
                "source_revival.memory_revival.active_memory_revival.active_relationship_graph"
                    .to_owned(),
                "source_revival.memory_revival.active_memory_revival.active_world_lore".to_owned(),
                "source_revival.memory_revival.active_memory_revival.agent_context_projection"
                    .to_owned(),
            ],
            use_rules: vec![
                "PromptContext is the only JSON packet the text backend may treat as prompt context.".to_owned(),
                "Broad source revival remains available for debug and console surfaces, not for narrative dispatch.".to_owned(),
                "Visible context may shape prose and choices; adjudication context may shape outcomes but must not be copied into player-visible text.".to_owned(),
                "Selected memory items are the only long-memory items physically present in the text prompt.".to_owned(),
            ],
        }
    }
}

/// Compile the physically narrow context packet sent to the text backend.
///
/// # Errors
///
/// Returns an error if any required source-revival section is missing from the
/// already assembled turn context packet.
pub fn assemble_prompt_context_packet(
    turn_context: &TurnContextPacket,
) -> Result<PromptContextPacket> {
    let source = &turn_context.source_revival;
    let memory = required_path(source, "/memory_revival")?;
    let active_memory = required_path(memory, "/active_memory_revival")?;
    let scene_pressure = parse_required_packet::<ScenePressurePacket>(
        memory,
        "/active_scene_pressure",
        "active_scene_pressure",
    )?;
    let body_resource = parse_required_packet::<BodyResourcePacket>(
        memory,
        "/active_body_resource_state",
        "active_body_resource_state",
    )?;
    let location_graph = parse_required_packet::<LocationGraphPacket>(
        memory,
        "/active_location_graph",
        "active_location_graph",
    )?;
    let private_context = parse_required_packet(
        source,
        "/private_adjudication_context",
        "private_adjudication_context",
    )?;
    let output_contract = parse_required_packet::<AgentOutputContract>(
        source,
        "/output_contract",
        "output_contract",
    )?;
    let affordance_graph = compile_affordance_graph_packet(
        turn_context.world_id.as_str(),
        turn_context.turn_id.as_str(),
        &scene_pressure,
        &body_resource,
        &location_graph,
    );
    let known_facts = required_path(memory, "/known_facts")?.clone();
    let selected_memory_items =
        required_path(active_memory, "/visible_prompt_revival/items")?.clone();
    let belief_graph = compile_belief_graph_packet(
        turn_context.world_id.as_str(),
        turn_context.turn_id.as_str(),
        &known_facts,
        &selected_memory_items,
    );
    let world_process_clock = compile_world_process_clock_packet(
        turn_context.world_id.as_str(),
        turn_context.turn_id.as_str(),
        &scene_pressure,
        &private_context,
    );
    let narrative_style_state = compile_narrative_style_state(
        turn_context.world_id.as_str(),
        turn_context.turn_id.as_str(),
        &output_contract,
    );

    Ok(PromptContextPacket {
        schema_version: PROMPT_CONTEXT_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: turn_context.world_id.clone(),
        turn_id: turn_context.turn_id.clone(),
        current_turn: required_path(source, "/current_turn")?.clone(),
        opening_randomizer: required_path(source, "/opening_randomizer")?.clone(),
        output_contract: serde_json::to_value(&output_contract)?,
        visible_context: PromptVisibleContext {
            recent_scene_window: required_path(memory, "/recent_scene_window")?.clone(),
            known_facts,
            active_scene_pressure: serde_json::to_value(&scene_pressure.visible_active)?,
            active_plot_threads: required_path(memory, "/active_plot_threads/active_visible")?
                .clone(),
            active_body_resource_state: serde_json::to_value(&body_resource)?,
            active_location_graph: serde_json::to_value(&location_graph)?,
            affordance_graph: serde_json::to_value(&affordance_graph)?,
            belief_graph: serde_json::to_value(&belief_graph)?,
            world_process_clock: serde_json::to_value(&world_process_clock.visible_processes)?,
            narrative_style_state: serde_json::to_value(&narrative_style_state)?,
            active_character_text_design: required_path(memory, "/active_character_text_design")?
                .clone(),
            selected_memory_items,
        },
        adjudication_context: PromptAdjudicationContext {
            private_adjudication_context: serde_json::to_value(&private_context)?,
            hidden_scene_pressure: required_path(
                memory,
                "/active_scene_pressure/hidden_adjudication_only",
            )?
            .clone(),
            hidden_world_process_clock: serde_json::to_value(
                &world_process_clock.adjudication_only_processes,
            )?,
            selected_adjudication_items: required_path(
                active_memory,
                "/adjudication_only_revival/items",
            )?
            .clone(),
        },
        source_of_truth_policy: required_path(source, "/source_of_truth_policy")?.clone(),
        prompt_policy: PromptContextPolicy::default(),
    })
}

fn required_path<'a>(value: &'a Value, path: &str) -> Result<&'a Value> {
    let Some(selected) = value.pointer(path) else {
        bail!("prompt context source missing required path: {path}");
    };
    Ok(selected)
}

fn parse_required_packet<T>(value: &Value, path: &str, label: &str) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(required_path(value, path)?.clone())
        .with_context(|| format!("prompt context source invalid packet at {label}"))
}

/// Extract and validate the prompt context JSON from a complete `WebGPT` prompt.
///
/// # Errors
///
/// Returns an error when the fenced prompt context JSON is missing or malformed.
pub fn extract_prompt_context_from_prompt(prompt: &str) -> Result<PromptContextPacket> {
    let marker = "prompt context packet JSON:";
    let marker_index = prompt
        .find(marker)
        .context("prompt context marker missing")?;
    let after_marker = &prompt[marker_index + marker.len()..];
    let fence_start = after_marker
        .find("```json")
        .context("prompt context JSON fence start missing")?;
    let after_fence = &after_marker[fence_start + "```json".len()..];
    let fence_end = after_fence
        .find("```")
        .context("prompt context JSON fence end missing")?;
    serde_json::from_str(after_fence[..fence_end].trim())
        .context("prompt context JSON parse failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[expect(
        clippy::too_many_lines,
        reason = "prompt context leakage fixture keeps the full source/debug shape visible"
    )]
    fn sample_turn_context() -> TurnContextPacket {
        TurnContextPacket {
            schema_version: crate::turn_context::TURN_CONTEXT_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_prompt".to_owned(),
            turn_id: "turn_0003".to_owned(),
            assembly_policy: crate::turn_context::TurnContextAssemblyPolicy::default(),
            source_revival: serde_json::json!({
                "schema_version": "singulari.agent_revival_packet.v1",
                "world_id": "stw_prompt",
                "turn_id": "turn_0003",
                "current_turn": {"player_input": "피한 눈을 살핀다"},
                "opening_randomizer": null,
                "output_contract": {
                    "language": "ko",
                    "must_return_json": true,
                    "hidden_truth_must_not_appear_in_visible_text": true,
                    "narrative_level": 1
                },
                "memory_revival": {
                    "resume_pack": {"chapters": ["debug-only old chapter"]},
                    "recent_scene_window": ["방금 길목에서 자루가 발견됐다."],
                    "known_facts": ["자루는 비어 있다."],
                    "active_scene_pressure": {
                        "schema_version": "singulari.scene_pressure_packet.v1",
                        "world_id": "stw_prompt",
                        "turn_id": "turn_0003",
                        "visible_active": [{
                            "schema_version": "singulari.scene_pressure.v1",
                            "pressure_id": "pressure:social:suspicion",
                            "kind": "social_permission",
                            "visibility": "player_visible",
                            "intensity": 3,
                            "urgency": "soon",
                            "source_refs": ["turn:0002"],
                            "observable_signals": ["낮아진 목소리"],
                            "choice_affordances": ["answer carefully"],
                            "prose_effect": {
                                "paragraph_pressure": "close",
                                "sensory_focus": ["voice"],
                                "dialogue_style": "short"
                            }
                        }],
                        "hidden_adjudication_only": [{
                            "schema_version": "singulari.scene_pressure.v1",
                            "pressure_id": "secret:timer",
                            "kind": "time_pressure",
                            "visibility": "hidden",
                            "intensity": 2,
                            "urgency": "ambient",
                            "source_refs": ["hidden_timer"],
                            "observable_signals": [],
                            "choice_affordances": [],
                            "prose_effect": {
                                "paragraph_pressure": "hidden",
                                "sensory_focus": [],
                                "dialogue_style": "hidden"
                            }
                        }],
                        "compiler_policy": {"source": "test", "visible_budget": 3, "hidden_budget": 2, "use_rules": []}
                    },
                    "active_plot_threads": {
                        "active_visible": [{"thread_id": "thread:last_holder"}]
                    },
                    "active_body_resource_state": {
                        "schema_version": "singulari.body_resource_packet.v1",
                        "world_id": "stw_prompt",
                        "turn_id": "turn_0003",
                        "body_constraints": [],
                        "resources": [],
                        "compiler_policy": {"source": "test", "use_rules": []}
                    },
                    "active_location_graph": {
                        "schema_version": "singulari.location_graph_packet.v1",
                        "world_id": "stw_prompt",
                        "turn_id": "turn_0003",
                        "current_location": null,
                        "known_nearby_locations": [],
                        "compiler_policy": {"source": "test", "nearby_location_budget": 3, "use_rules": []}
                    },
                    "active_character_text_design": {"active_designs": []},
                    "active_memory_revival": {
                        "player_visible_archive_view": {
                            "entries": [{"summary": "debug archive must stay out of prompt context"}]
                        },
                        "query_recall": {"hits": [{"source_id": "rel:stale"}]},
                        "active_relationship_graph": {
                            "active_edges": [{"visible_summary": "stale full graph edge"}]
                        },
                        "active_world_lore": {
                            "entries": [{"summary": "stale full lore"}]
                        },
                        "visible_prompt_revival": {
                            "items": [{
                                "source_kind": "relationship_edge",
                                "source_id": "rel:current",
                                "payload": {"visible_summary": "current selected edge"}
                            }]
                        },
                        "adjudication_only_revival": {
                            "items": [{"source_id": "secret:selected"}]
                        },
                        "agent_context_projection": {
                            "relationship_summaries": [{"target": "rel:debug"}]
                        }
                    }
                },
                "private_adjudication_context": {
                    "hidden_timers": [{
                        "timer_id": "secret:timer",
                        "kind": "secret_clock",
                        "remaining_turns": 2,
                        "effect": "secret timer"
                    }],
                    "unrevealed_constraints": [],
                    "plausibility_gates": []
                },
                "source_of_truth_policy": {
                    "world_state_source": "world_store",
                    "conflict_rule": "revival_packet_wins"
                }
            }),
        }
    }

    #[test]
    fn prompt_context_excludes_debug_revival_sections() -> anyhow::Result<()> {
        let context = assemble_prompt_context_packet(&sample_turn_context())?;
        let serialized = serde_json::to_string(&context)?;

        assert!(serialized.contains("current selected edge"));
        assert!(!serialized.contains("debug archive must stay out of prompt context"));
        assert!(!serialized.contains("stale full graph edge"));
        assert!(!serialized.contains("stale full lore"));
        assert!(!serialized.contains("relationship_summaries"));
        Ok(())
    }

    #[test]
    fn prompt_context_keeps_adjudication_items_out_of_visible_context() -> anyhow::Result<()> {
        let context = assemble_prompt_context_packet(&sample_turn_context())?;
        let visible = serde_json::to_string(&context.visible_context)?;
        let adjudication = serde_json::to_string(&context.adjudication_context)?;

        assert!(!visible.contains("secret:selected"));
        assert!(!visible.contains("secret timer"));
        assert!(adjudication.contains("secret:selected"));
        assert!(adjudication.contains("secret timer"));
        Ok(())
    }
}
