#![allow(clippy::missing_errors_doc)]

use crate::agent_bridge::{AgentTurnResponse, PendingAgentChoice};
use crate::resolution::{
    FreeformGateTrace, ResolutionOutcomeKind, freeform_gate_trace_from_proposal,
};
use crate::store::{append_jsonl, read_json, write_json};
use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const PLAYER_INTENT_TRACE_SCHEMA_VERSION: &str = "singulari.player_intent_trace.v1";
pub const PLAYER_INTENT_EVENT_SCHEMA_VERSION: &str = "singulari.player_intent_event.v1";
pub const PLAYER_INTENT_TRACE_FILENAME: &str = "player_intent_trace.json";
pub const PLAYER_INTENT_EVENTS_FILENAME: &str = "player_intent_events.jsonl";

const ACTIVE_INTENT_BUDGET: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlayerIntentTracePacket {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub active_intents: Vec<PlayerIntentEventRecord>,
    pub compiler_policy: PlayerIntentPolicy,
}

impl Default for PlayerIntentTracePacket {
    fn default() -> Self {
        Self {
            schema_version: PLAYER_INTENT_TRACE_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            turn_id: String::new(),
            active_intents: Vec::new(),
            compiler_policy: PlayerIntentPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlayerIntentEventPlan {
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub records: Vec<PlayerIntentEventRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlayerIntentEventRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub event_id: String,
    pub scope: String,
    pub intent_shape: String,
    pub evidence: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution_intent_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution_outcome: Option<ResolutionOutcomeKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freeform_gate_trace: Option<FreeformGateTrace>,
    pub expires_after_turns: u8,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlayerIntentPolicy {
    pub source: String,
    pub active_intent_budget: usize,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for PlayerIntentPolicy {
    fn default() -> Self {
        Self {
            source: "compiled_from_player_inputs_and_choice_shapes_v1".to_owned(),
            active_intent_budget: ACTIVE_INTENT_BUDGET,
            use_rules: vec![
                "Intent trace is scene pressure, not a permanent player profile.".to_owned(),
                "Expire intent quickly unless repeated evidence keeps it active.".to_owned(),
                "Use intent to bias affordance wording, not to remove player freedom.".to_owned(),
            ],
        }
    }
}

#[must_use]
pub fn prepare_player_intent_event_plan(
    world_id: &str,
    turn_id: &str,
    player_input: &str,
    selected_choice: Option<&PendingAgentChoice>,
    response: &AgentTurnResponse,
) -> PlayerIntentEventPlan {
    let intent_shape = selected_choice.map_or_else(
        || "freeform_or_unmatched".to_owned(),
        |choice| format!("slot:{}:{}", choice.slot, choice.tag),
    );
    let evidence = if player_input.trim().is_empty() {
        response
            .visible_scene
            .text_blocks
            .first()
            .cloned()
            .unwrap_or_default()
    } else {
        player_input.trim().to_owned()
    };
    let resolution_intent_summary = response
        .resolution_proposal
        .as_ref()
        .map(|proposal| proposal.interpreted_intent.summary.clone());
    let resolution_outcome = response
        .resolution_proposal
        .as_ref()
        .map(|proposal| proposal.outcome.kind);
    let freeform_gate_trace = response
        .resolution_proposal
        .as_ref()
        .and_then(|proposal| freeform_gate_trace_from_proposal(player_input, proposal));
    PlayerIntentEventPlan {
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        records: vec![PlayerIntentEventRecord {
            schema_version: PLAYER_INTENT_EVENT_SCHEMA_VERSION.to_owned(),
            world_id: world_id.to_owned(),
            turn_id: turn_id.to_owned(),
            event_id: format!("player_intent_event:{turn_id}:00"),
            scope: "scene".to_owned(),
            intent_shape,
            evidence,
            resolution_intent_summary,
            resolution_outcome,
            freeform_gate_trace,
            expires_after_turns: 3,
            recorded_at: Utc::now().to_rfc3339(),
        }],
    }
}

pub fn append_player_intent_event_plan(
    world_dir: &Path,
    plan: &PlayerIntentEventPlan,
) -> Result<()> {
    for record in &plan.records {
        append_jsonl(&world_dir.join(PLAYER_INTENT_EVENTS_FILENAME), record)?;
    }
    Ok(())
}

pub fn rebuild_player_intent_trace(
    world_dir: &Path,
    base_packet: &PlayerIntentTracePacket,
) -> Result<PlayerIntentTracePacket> {
    let mut active_intents = load_player_intent_event_records(world_dir)?;
    active_intents.reverse();
    active_intents.truncate(ACTIVE_INTENT_BUDGET);
    active_intents.reverse();
    let packet = PlayerIntentTracePacket {
        schema_version: PLAYER_INTENT_TRACE_SCHEMA_VERSION.to_owned(),
        world_id: base_packet.world_id.clone(),
        turn_id: base_packet.turn_id.clone(),
        active_intents,
        compiler_policy: PlayerIntentPolicy::default(),
    };
    write_json(&world_dir.join(PLAYER_INTENT_TRACE_FILENAME), &packet)?;
    Ok(packet)
}

pub fn load_player_intent_trace_state(
    world_dir: &Path,
    base_packet: PlayerIntentTracePacket,
) -> Result<PlayerIntentTracePacket> {
    let path = world_dir.join(PLAYER_INTENT_TRACE_FILENAME);
    if path.is_file() {
        return read_json(&path);
    }
    Ok(base_packet)
}

fn load_player_intent_event_records(world_dir: &Path) -> Result<Vec<PlayerIntentEventRecord>> {
    let path = world_dir.join(PLAYER_INTENT_EVENTS_FILENAME);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)?;
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str::<PlayerIntentEventRecord>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_bridge::AGENT_TURN_RESPONSE_SCHEMA_VERSION;
    use crate::models::{NARRATIVE_SCENE_SCHEMA_VERSION, NarrativeScene};
    use crate::resolution::{
        ActionAmbiguity, ActionInputKind, ActionIntent, GateKind, GateResult, GateStatus,
        NarrativeBrief, RESOLUTION_PROPOSAL_SCHEMA_VERSION, ResolutionOutcome,
        ResolutionOutcomeKind, ResolutionProposal, ResolutionVisibility,
    };

    #[test]
    fn materializes_scene_scoped_intent_trace() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let response = AgentTurnResponse {
            schema_version: AGENT_TURN_RESPONSE_SCHEMA_VERSION.to_owned(),
            world_id: "stw_intent".to_owned(),
            turn_id: "turn_0001".to_owned(),
            resolution_proposal: None,
            scene_director_proposal: None,
            consequence_proposal: None,
            social_exchange_proposal: None,
            visible_scene: NarrativeScene {
                schema_version: NARRATIVE_SCENE_SCHEMA_VERSION.to_owned(),
                speaker: None,
                text_blocks: Vec::new(),
                tone_notes: Vec::new(),
            },
            adjudication: None,
            canon_event: None,
            entity_updates: Vec::new(),
            relationship_updates: Vec::new(),
            plot_thread_events: Vec::new(),
            scene_pressure_events: Vec::new(),
            world_lore_updates: Vec::new(),
            character_text_design_updates: Vec::new(),
            body_resource_events: Vec::new(),
            location_events: Vec::new(),
            extra_contacts: Vec::new(),
            hidden_state_delta: Vec::new(),
            needs_context: Vec::new(),
            next_choices: Vec::new(),
            actor_goal_events: Vec::new(),
            actor_move_events: Vec::new(),
        };
        let plan = prepare_player_intent_event_plan(
            "stw_intent",
            "turn_0001",
            "6 조용히 듣는다",
            None,
            &response,
        );
        append_player_intent_event_plan(temp.path(), &plan)?;
        let packet = rebuild_player_intent_trace(
            temp.path(),
            &PlayerIntentTracePacket {
                world_id: "stw_intent".to_owned(),
                turn_id: "turn_0001".to_owned(),
                ..PlayerIntentTracePacket::default()
            },
        )?;
        assert_eq!(packet.active_intents.len(), 1);
        assert_eq!(packet.active_intents[0].scope, "scene");
        Ok(())
    }

    #[test]
    fn freeform_resolution_proposal_materializes_gate_trace() {
        let response = AgentTurnResponse {
            schema_version: AGENT_TURN_RESPONSE_SCHEMA_VERSION.to_owned(),
            world_id: "stw_intent_resolution".to_owned(),
            turn_id: "turn_0002".to_owned(),
            resolution_proposal: Some(ResolutionProposal {
                schema_version: RESOLUTION_PROPOSAL_SCHEMA_VERSION.to_owned(),
                world_id: "stw_intent_resolution".to_owned(),
                turn_id: "turn_0002".to_owned(),
                interpreted_intent: ActionIntent {
                    input_kind: ActionInputKind::Freeform,
                    summary: "문지기를 말로 설득한다.".to_owned(),
                    target_refs: vec!["pressure:social:gate".to_owned()],
                    pressure_refs: vec!["pressure:social:gate".to_owned()],
                    evidence_refs: vec!["current_turn".to_owned()],
                    ambiguity: ActionAmbiguity::Minor,
                },
                outcome: ResolutionOutcome {
                    kind: ResolutionOutcomeKind::PartialSuccess,
                    summary: "대화의 틈이 생긴다.".to_owned(),
                    evidence_refs: vec!["current_turn".to_owned()],
                },
                gate_results: vec![GateResult {
                    gate_kind: GateKind::SocialPermission,
                    gate_ref: "pressure:social:gate".to_owned(),
                    visibility: ResolutionVisibility::PlayerVisible,
                    status: GateStatus::Softened,
                    reason: "의심이 완전히 풀리지는 않았다.".to_owned(),
                    evidence_refs: vec!["current_turn".to_owned()],
                }],
                proposed_effects: Vec::new(),
                process_ticks: Vec::new(),
                narrative_brief: NarrativeBrief {
                    visible_summary: "말끝에 틈이 생긴다.".to_owned(),
                    required_beats: Vec::new(),
                    forbidden_visible_details: Vec::new(),
                },
                next_choice_plan: Vec::new(),
            }),
            scene_director_proposal: None,
            consequence_proposal: None,
            social_exchange_proposal: None,
            visible_scene: NarrativeScene {
                schema_version: NARRATIVE_SCENE_SCHEMA_VERSION.to_owned(),
                speaker: None,
                text_blocks: vec!["문지기가 말을 끊지 않았다.".to_owned()],
                tone_notes: Vec::new(),
            },
            adjudication: None,
            canon_event: None,
            entity_updates: Vec::new(),
            relationship_updates: Vec::new(),
            plot_thread_events: Vec::new(),
            scene_pressure_events: Vec::new(),
            world_lore_updates: Vec::new(),
            character_text_design_updates: Vec::new(),
            body_resource_events: Vec::new(),
            location_events: Vec::new(),
            extra_contacts: Vec::new(),
            hidden_state_delta: Vec::new(),
            needs_context: Vec::new(),
            next_choices: Vec::new(),
            actor_goal_events: Vec::new(),
            actor_move_events: Vec::new(),
        };

        let plan = prepare_player_intent_event_plan(
            "stw_intent_resolution",
            "turn_0002",
            "6 문지기에게 둘러댄다",
            None,
            &response,
        );
        let record = &plan.records[0];

        assert_eq!(
            record.resolution_outcome,
            Some(ResolutionOutcomeKind::PartialSuccess)
        );
        assert_eq!(
            record.resolution_intent_summary.as_deref(),
            Some("문지기를 말로 설득한다.")
        );
        let Some(trace) = &record.freeform_gate_trace else {
            panic!("freeform resolution should create a gate trace");
        };
        assert_eq!(trace.gate_results.len(), 1);
        assert_eq!(trace.final_outcome, ResolutionOutcomeKind::PartialSuccess);
    }
}
