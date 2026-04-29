use crate::affordance_graph::{AffordanceGraphPacket, compile_affordance_graph_packet};
use crate::agent_bridge::{AgentOutputContract, PendingAgentTurn};
use crate::belief_graph::{BeliefGraphPacket, compile_belief_graph_packet};
use crate::body_resource::BodyResourcePacket;
use crate::location_graph::LocationGraphPacket;
use crate::prompt_context_budget::{
    PromptContextBudgetReport, PromptContextBudgetReportSource,
    compile_prompt_context_budget_report,
};
use crate::revival::{AgentRevivalCompileOptions, build_agent_revival_packet};
use crate::scene_director::{
    SceneDirectorCompileInput, SceneDirectorPacket, compile_scene_director_packet_from_input,
    merge_scene_director_history,
};
use crate::scene_pressure::ScenePressurePacket;
use crate::turn_context::{TurnContextPacket, assemble_turn_context_packet};
use crate::world_process_clock::{WorldProcessClockPacket, compile_world_process_clock_packet};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

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
    pub budget_report: PromptContextBudgetReport,
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
    pub active_scene_director: Value,
    pub narrative_style_state: Value,
    pub active_character_text_design: Value,
    pub active_change_ledger: Value,
    pub active_pattern_debt: Value,
    pub active_belief_graph: Value,
    pub active_world_process_clock: Value,
    pub active_actor_agency: Value,
    pub active_player_intent_trace: Value,
    pub active_turn_retrieval_controller: Value,
    pub selected_context_capsules: Value,
    pub active_autobiographical_index: Value,
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
                "Selected context capsules replace covered broad prompt projections; broad projection files remain source-of-truth/debug surfaces.".to_owned(),
            ],
        }
    }
}

pub struct CompilePromptContextPacketOptions<'a> {
    pub store_root: Option<&'a Path>,
    pub pending: &'a PendingAgentTurn,
    pub engine_session_kind: &'a str,
}

/// Compile the backend-facing context packet for one pending turn.
///
/// This is the boundary that may read broad source-revival material. Text
/// backend adapters should receive the returned `PromptContextPacket`, not the
/// source revival packet.
///
/// # Errors
///
/// Returns an error when source revival cannot be built or required prompt
/// context sections are missing.
pub fn compile_prompt_context_packet(
    options: &CompilePromptContextPacketOptions<'_>,
) -> Result<PromptContextPacket> {
    let source_revival_packet = build_agent_revival_packet(&AgentRevivalCompileOptions {
        store_root: options.store_root,
        pending: options.pending,
        engine_session_kind: options.engine_session_kind,
    })?;
    let turn_context_packet = assemble_turn_context_packet(options.pending, source_revival_packet);
    assemble_prompt_context_packet(&turn_context_packet)
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
    let prompt_memory = load_prompt_memory_values(memory, active_memory)?;
    let derived = compile_derived_context_packets(CompileDerivedContextSource {
        turn_context,
        known_facts: &prompt_memory.known_facts,
        selected_memory_items: &prompt_memory.selected_memory_items,
        scene_pressure: &scene_pressure,
        active_plot_threads: &prompt_memory.active_plot_threads,
        active_scene_director: prompt_memory.active_scene_director.as_ref(),
        active_pattern_debt: &prompt_memory.active_pattern_debt,
        active_actor_agency: &prompt_memory.active_actor_agency,
        active_world_process_clock: &prompt_memory.active_world_process_clock,
        active_player_intent_trace: &prompt_memory.active_player_intent_trace,
        recent_scene_window: required_path(memory, "/recent_scene_window")?,
        body_resource: &body_resource,
        location_graph: &location_graph,
        private_context: &private_context,
    });
    let prompt_policy = PromptContextPolicy::default();
    let budget_report = compile_prompt_budget_report(CompilePromptBudgetSource {
        turn_context,
        selected_memory_items: &prompt_memory.selected_memory_items,
        affordance_graph: &derived.affordance_graph,
        belief_graph: &derived.belief_graph,
        world_process_clock: &derived.world_process_clock,
        active_change_ledger: &prompt_memory.active_change_ledger,
        active_pattern_debt: &prompt_memory.active_pattern_debt,
        selected_context_capsules: &prompt_memory.selected_context_capsules,
        prompt_policy: &prompt_policy,
    })?;

    let visible_context = compile_visible_context(VisibleContextSource {
        memory,
        prompt_memory,
        scene_pressure: &scene_pressure,
        body_resource: &body_resource,
        location_graph: &location_graph,
        affordance_graph: &derived.affordance_graph,
        belief_graph: &derived.belief_graph,
        world_process_clock: &derived.world_process_clock,
        scene_director: &derived.scene_director,
    })?;
    let adjudication_context = compile_adjudication_context(AdjudicationContextSource {
        memory,
        active_memory,
        private_context: &private_context,
        world_process_clock: &derived.world_process_clock,
    })?;

    Ok(PromptContextPacket {
        schema_version: PROMPT_CONTEXT_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: turn_context.world_id.clone(),
        turn_id: turn_context.turn_id.clone(),
        current_turn: required_path(source, "/current_turn")?.clone(),
        opening_randomizer: required_path(source, "/opening_randomizer")?.clone(),
        output_contract: serde_json::to_value(&output_contract)?,
        visible_context,
        adjudication_context,
        source_of_truth_policy: required_path(source, "/source_of_truth_policy")?.clone(),
        prompt_policy,
        budget_report,
    })
}

struct PromptMemoryValues {
    known_facts: Value,
    active_plot_threads: Value,
    active_scene_director: Option<SceneDirectorPacket>,
    active_change_ledger: Value,
    active_pattern_debt: Value,
    active_belief_graph: Value,
    active_world_process_clock: Value,
    active_actor_agency: Value,
    active_player_intent_trace: Value,
    active_narrative_style_state: Value,
    active_turn_retrieval_controller: Value,
    selected_context_capsules: Value,
    active_autobiographical_index: Value,
    selected_memory_items: Value,
}

fn load_prompt_memory_values(memory: &Value, active_memory: &Value) -> Result<PromptMemoryValues> {
    Ok(PromptMemoryValues {
        known_facts: required_path(memory, "/known_facts")?.clone(),
        active_plot_threads: required_path(memory, "/active_plot_threads")?.clone(),
        active_scene_director: memory
            .pointer("/active_scene_director")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok()),
        active_change_ledger: required_path(memory, "/active_change_ledger")?.clone(),
        active_pattern_debt: required_path(memory, "/active_pattern_debt")?.clone(),
        active_belief_graph: required_path(memory, "/active_belief_graph")?.clone(),
        active_world_process_clock: required_path(memory, "/active_world_process_clock")?.clone(),
        active_actor_agency: memory
            .pointer("/active_actor_agency")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({"active_goals": [], "recent_moves": []})),
        active_player_intent_trace: required_path(memory, "/active_player_intent_trace")?.clone(),
        active_narrative_style_state: required_path(memory, "/active_narrative_style_state")?
            .clone(),
        active_turn_retrieval_controller: required_path(
            memory,
            "/active_turn_retrieval_controller",
        )?
        .clone(),
        selected_context_capsules: required_path(memory, "/selected_context_capsules")?.clone(),
        active_autobiographical_index: required_path(memory, "/active_autobiographical_index")?
            .clone(),
        selected_memory_items: required_path(active_memory, "/visible_prompt_revival/items")?
            .clone(),
    })
}

struct DerivedContextPackets {
    affordance_graph: AffordanceGraphPacket,
    belief_graph: BeliefGraphPacket,
    world_process_clock: WorldProcessClockPacket,
    scene_director: SceneDirectorPacket,
}

#[derive(Clone, Copy)]
struct CompileDerivedContextSource<'a> {
    turn_context: &'a TurnContextPacket,
    known_facts: &'a Value,
    selected_memory_items: &'a Value,
    scene_pressure: &'a ScenePressurePacket,
    active_plot_threads: &'a Value,
    active_scene_director: Option<&'a SceneDirectorPacket>,
    active_pattern_debt: &'a Value,
    active_actor_agency: &'a Value,
    active_world_process_clock: &'a Value,
    active_player_intent_trace: &'a Value,
    recent_scene_window: &'a Value,
    body_resource: &'a BodyResourcePacket,
    location_graph: &'a LocationGraphPacket,
    private_context: &'a crate::agent_bridge::AgentPrivateAdjudicationContext,
}

fn compile_derived_context_packets(
    source: CompileDerivedContextSource<'_>,
) -> DerivedContextPackets {
    let affordance_graph = compile_affordance_graph_packet(
        source.turn_context.world_id.as_str(),
        source.turn_context.turn_id.as_str(),
        source.scene_pressure,
        source.body_resource,
        source.location_graph,
    );
    let belief_graph = compile_belief_graph_packet(
        source.turn_context.world_id.as_str(),
        source.turn_context.turn_id.as_str(),
        source.known_facts,
        source.selected_memory_items,
    );
    let world_process_clock = compile_world_process_clock_packet(
        source.turn_context.world_id.as_str(),
        source.turn_context.turn_id.as_str(),
        source.scene_pressure,
        source.private_context,
    );
    let world_process_clock_value = serde_json::to_value(&world_process_clock)
        .unwrap_or_else(|_| source.active_world_process_clock.clone());
    let scene_director = merge_scene_director_history(
        compile_scene_director_packet_from_input(SceneDirectorCompileInput {
            world_id: source.turn_context.world_id.as_str(),
            turn_id: source.turn_context.turn_id.as_str(),
            scene_pressure: source.scene_pressure,
            active_pattern_debt: source.active_pattern_debt,
            active_plot_threads: Some(source.active_plot_threads),
            active_actor_agency: Some(source.active_actor_agency),
            active_world_process_clock: Some(&world_process_clock_value),
            active_player_intent_trace: Some(source.active_player_intent_trace),
            recent_scene_window: Some(source.recent_scene_window),
        }),
        source.active_scene_director,
    );
    DerivedContextPackets {
        affordance_graph,
        belief_graph,
        world_process_clock,
        scene_director,
    }
}

#[derive(Clone, Copy)]
struct CompilePromptBudgetSource<'a> {
    turn_context: &'a TurnContextPacket,
    selected_memory_items: &'a Value,
    affordance_graph: &'a AffordanceGraphPacket,
    belief_graph: &'a BeliefGraphPacket,
    world_process_clock: &'a WorldProcessClockPacket,
    active_change_ledger: &'a Value,
    active_pattern_debt: &'a Value,
    selected_context_capsules: &'a Value,
    prompt_policy: &'a PromptContextPolicy,
}

fn compile_prompt_budget_report(
    source: CompilePromptBudgetSource<'_>,
) -> Result<PromptContextBudgetReport> {
    compile_prompt_context_budget_report(PromptContextBudgetReportSource {
        world_id: source.turn_context.world_id.as_str(),
        turn_id: source.turn_context.turn_id.as_str(),
        selected_memory_items: source.selected_memory_items,
        affordance_graph: source.affordance_graph,
        belief_graph: source.belief_graph,
        world_process_clock: source.world_process_clock,
        active_change_ledger: source.active_change_ledger,
        active_pattern_debt: source.active_pattern_debt,
        selected_context_capsules: source.selected_context_capsules,
        omitted_debug_sections: &source.prompt_policy.omitted_debug_sections,
    })
}

struct VisibleContextSource<'a> {
    memory: &'a Value,
    prompt_memory: PromptMemoryValues,
    scene_pressure: &'a ScenePressurePacket,
    body_resource: &'a BodyResourcePacket,
    location_graph: &'a LocationGraphPacket,
    affordance_graph: &'a AffordanceGraphPacket,
    belief_graph: &'a BeliefGraphPacket,
    world_process_clock: &'a WorldProcessClockPacket,
    scene_director: &'a SceneDirectorPacket,
}

fn compile_visible_context(source: VisibleContextSource<'_>) -> Result<PromptVisibleContext> {
    Ok(PromptVisibleContext {
        recent_scene_window: required_path(source.memory, "/recent_scene_window")?.clone(),
        known_facts: source.prompt_memory.known_facts,
        active_scene_pressure: serde_json::to_value(&source.scene_pressure.visible_active)?,
        active_plot_threads: required_path(
            &source.prompt_memory.active_plot_threads,
            "/active_visible",
        )?
        .clone(),
        active_body_resource_state: serde_json::to_value(source.body_resource)?,
        active_location_graph: serde_json::to_value(source.location_graph)?,
        affordance_graph: serde_json::to_value(source.affordance_graph)?,
        belief_graph: serde_json::to_value(source.belief_graph)?,
        world_process_clock: serde_json::to_value(&source.world_process_clock.visible_processes)?,
        active_scene_director: serde_json::to_value(source.scene_director)?,
        active_character_text_design: capsule_covered_prompt_projection(
            "active_character_text_design",
            "character_text_design",
            required_path(source.memory, "/active_character_text_design")?,
            &source.prompt_memory.selected_context_capsules,
        ),
        active_change_ledger: source.prompt_memory.active_change_ledger,
        active_pattern_debt: source.prompt_memory.active_pattern_debt,
        active_belief_graph: source.prompt_memory.active_belief_graph,
        active_world_process_clock: source.prompt_memory.active_world_process_clock,
        active_actor_agency: source.prompt_memory.active_actor_agency,
        active_player_intent_trace: source.prompt_memory.active_player_intent_trace,
        active_turn_retrieval_controller: source.prompt_memory.active_turn_retrieval_controller,
        selected_context_capsules: source.prompt_memory.selected_context_capsules,
        active_autobiographical_index: source.prompt_memory.active_autobiographical_index,
        narrative_style_state: source.prompt_memory.active_narrative_style_state,
        selected_memory_items: source.prompt_memory.selected_memory_items,
    })
}

fn capsule_covered_prompt_projection(
    section: &str,
    capsule_kind: &str,
    full_projection: &Value,
    selected_context_capsules: &Value,
) -> Value {
    if !has_selected_capsule_kind(selected_context_capsules, capsule_kind) {
        return full_projection.clone();
    }
    serde_json::json!({
        "covered_by_selected_context_capsules": true,
        "section": section,
        "capsule_kind": capsule_kind,
        "source": "visible_context.selected_context_capsules.selected_capsules",
    })
}

fn has_selected_capsule_kind(selected_context_capsules: &Value, capsule_kind: &str) -> bool {
    selected_context_capsules
        .pointer("/selected_capsules")
        .and_then(Value::as_array)
        .is_some_and(|capsules| {
            capsules.iter().any(|capsule| {
                capsule
                    .get("kind")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind == capsule_kind)
            })
        })
}

#[derive(Clone, Copy)]
struct AdjudicationContextSource<'a> {
    memory: &'a Value,
    active_memory: &'a Value,
    private_context: &'a crate::agent_bridge::AgentPrivateAdjudicationContext,
    world_process_clock: &'a WorldProcessClockPacket,
}

fn compile_adjudication_context(
    source: AdjudicationContextSource<'_>,
) -> Result<PromptAdjudicationContext> {
    Ok(PromptAdjudicationContext {
        private_adjudication_context: serde_json::to_value(source.private_context)?,
        hidden_scene_pressure: required_path(
            source.memory,
            "/active_scene_pressure/hidden_adjudication_only",
        )?
        .clone(),
        hidden_world_process_clock: serde_json::to_value(
            &source.world_process_clock.adjudication_only_processes,
        )?,
        selected_adjudication_items: required_path(
            source.active_memory,
            "/adjudication_only_revival/items",
        )?
        .clone(),
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
                    "active_world_lore": {"entries": []},
                    "active_relationship_graph": {"active_edges": []},
                    "active_change_ledger": {"active_changes": []},
                    "active_pattern_debt": {"active_patterns": []},
                    "active_belief_graph": {"protagonist_visible_beliefs": []},
                    "active_world_process_clock": {"visible_processes": [], "adjudication_only_processes": []},
                    "active_actor_agency": {"active_goals": [], "recent_moves": []},
                    "active_player_intent_trace": {"active_intents": []},
                    "active_narrative_style_state": {"active_style_events": []},
                    "active_turn_retrieval_controller": {"active_goals": [], "active_role_stance": [], "retrieval_cues": []},
                    "selected_context_capsules": {"selected_capsules": [], "budget_report": {}},
                    "active_autobiographical_index": {"periods": [], "general_events": []},
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

    #[test]
    fn prompt_context_includes_player_visible_scene_director_packet() -> anyhow::Result<()> {
        let context = assemble_prompt_context_packet(&sample_turn_context())?;
        let director = &context.visible_context.active_scene_director;

        assert_eq!(
            director
                .pointer("/schema_version")
                .and_then(serde_json::Value::as_str),
            Some(crate::scene_director::SCENE_DIRECTOR_PACKET_SCHEMA_VERSION)
        );
        assert_eq!(
            director
                .pointer("/compiler_policy/mode")
                .and_then(serde_json::Value::as_str),
            Some("advisory_only")
        );
        assert!(!serde_json::to_string(director)?.contains("secret timer"));
        Ok(())
    }

    #[test]
    fn prompt_context_reports_budget_reasons_for_included_and_excluded_sections()
    -> anyhow::Result<()> {
        let context = assemble_prompt_context_packet(&sample_turn_context())?;
        let report = &context.budget_report;

        assert_eq!(
            report.schema_version,
            crate::prompt_context_budget::PROMPT_CONTEXT_BUDGET_REPORT_SCHEMA_VERSION
        );
        assert_eq!(report.budgets["selected_memory_items"].used, 1);
        assert!(report.included.iter().any(|entry| {
            entry.section == "visible_context.selected_memory_items"
                && entry.source_id == "rel:current"
        }));
        assert!(report.excluded.iter().any(|entry| {
            entry
                .section
                .ends_with("active_memory_revival.active_relationship_graph")
        }));
        Ok(())
    }

    #[test]
    fn selected_context_capsule_replaces_covered_character_text_projection() -> anyhow::Result<()> {
        let mut turn_context = sample_turn_context();
        turn_context.source_revival["memory_revival"]["active_character_text_design"] = serde_json::json!({
            "active_designs": [{
                "entity_id": "char:gate_guard",
                "visible_name": "Gate Guard",
                "speech": ["full broad projection should not be sent"]
            }]
        });
        turn_context.source_revival["memory_revival"]["selected_context_capsules"] = serde_json::json!({
            "selected_capsules": [{
                "capsule_id": "character_text:char_gate_guard",
                "kind": "character_text_design",
                "reason": "current_goal_match",
                "body": {
                    "payload": {
                        "entity_id": "char:gate_guard",
                        "speech": ["capsule speech only"]
                    }
                }
            }],
            "rejected_capsules": [],
            "budget_report": {}
        });

        let context = assemble_prompt_context_packet(&turn_context)?;
        let serialized = serde_json::to_string(&context.visible_context)?;

        assert!(serialized.contains("capsule speech only"));
        assert!(serialized.contains("covered_by_selected_context_capsules"));
        assert!(!serialized.contains("full broad projection should not be sent"));
        Ok(())
    }
}
