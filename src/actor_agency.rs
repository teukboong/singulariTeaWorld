#![allow(clippy::missing_errors_doc)]

use crate::agent_bridge::AgentTurnResponse;
use crate::relationship_graph::RelationshipGraphPacket;
use crate::response_context::ContextVisibility;
use crate::store::{append_jsonl, read_json, write_json};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const ACTOR_AGENCY_PACKET_SCHEMA_VERSION: &str = "singulari.actor_agency_packet.v1";
pub const ACTOR_GOAL_SCHEMA_VERSION: &str = "singulari.actor_goal.v1";
pub const ACTOR_MOVE_SCHEMA_VERSION: &str = "singulari.actor_move.v1";
pub const ACTOR_GOAL_EVENT_SCHEMA_VERSION: &str = "singulari.actor_goal_event.v1";
pub const ACTOR_MOVE_EVENT_SCHEMA_VERSION: &str = "singulari.actor_move_event.v1";
pub const ACTOR_AGENCY_FILENAME: &str = "actor_agency.json";
pub const ACTOR_GOAL_EVENTS_FILENAME: &str = "actor_goal_events.jsonl";
pub const ACTOR_MOVE_EVENTS_FILENAME: &str = "actor_move_events.jsonl";

const ACTIVE_ACTOR_GOAL_BUDGET: usize = 12;
const RECENT_ACTOR_MOVE_BUDGET: usize = 16;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorAgencyPacket {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub active_goals: Vec<ActorGoal>,
    #[serde(default)]
    pub recent_moves: Vec<ActorMove>,
    pub compiler_policy: ActorAgencyPolicy,
}

impl Default for ActorAgencyPacket {
    fn default() -> Self {
        Self {
            schema_version: ACTOR_AGENCY_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            turn_id: String::new(),
            active_goals: Vec::new(),
            recent_moves: Vec::new(),
            compiler_policy: ActorAgencyPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorGoal {
    pub schema_version: String,
    pub actor_ref: String,
    pub goal_id: String,
    pub visibility: ContextVisibility,
    pub desire: String,
    pub fear_or_constraint: String,
    #[serde(default)]
    pub current_leverage: Vec<String>,
    #[serde(default)]
    pub pressure_refs: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub source_event_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorMove {
    pub schema_version: String,
    pub actor_ref: String,
    pub move_id: String,
    pub visibility: ContextVisibility,
    pub action_summary: String,
    #[serde(default)]
    pub produced_pressure_refs: Vec<String>,
    #[serde(default)]
    pub relationship_refs: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub source_event_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorAgencyPolicy {
    pub source: String,
    pub active_goal_budget: usize,
    pub recent_move_budget: usize,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for ActorAgencyPolicy {
    fn default() -> Self {
        Self {
            source: "materialized_from_actor_goal_and_move_events_v1".to_owned(),
            active_goal_budget: ACTIVE_ACTOR_GOAL_BUDGET,
            recent_move_budget: RECENT_ACTOR_MOVE_BUDGET,
            use_rules: vec![
                "Actor goals are bounded local pressures, not full autonomous planners.".to_owned(),
                "Hidden goals may shape adjudication but must not appear in visible prose."
                    .to_owned(),
                "Visible moves are observable behavior; motives must be inferred through evidence."
                    .to_owned(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentActorGoalUpdate {
    pub actor_ref: String,
    pub goal_id: String,
    pub visibility: ContextVisibility,
    pub desire: String,
    pub fear_or_constraint: String,
    #[serde(default)]
    pub current_leverage: Vec<String>,
    #[serde(default)]
    pub pressure_refs: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub retired: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentActorMoveUpdate {
    pub actor_ref: String,
    pub move_id: String,
    pub visibility: ContextVisibility,
    pub action_summary: String,
    #[serde(default)]
    pub produced_pressure_refs: Vec<String>,
    #[serde(default)]
    pub relationship_refs: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorAgencyEventPlan {
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub goal_records: Vec<ActorGoalEventRecord>,
    #[serde(default)]
    pub move_records: Vec<ActorMoveEventRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorGoalEventRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub event_id: String,
    pub actor_ref: String,
    pub goal_id: String,
    pub visibility: ContextVisibility,
    pub desire: String,
    pub fear_or_constraint: String,
    #[serde(default)]
    pub current_leverage: Vec<String>,
    #[serde(default)]
    pub pressure_refs: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub retired: bool,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorMoveEventRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub event_id: String,
    pub actor_ref: String,
    pub move_id: String,
    pub visibility: ContextVisibility,
    pub action_summary: String,
    #[serde(default)]
    pub produced_pressure_refs: Vec<String>,
    #[serde(default)]
    pub relationship_refs: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub recorded_at: String,
}

pub fn prepare_actor_agency_event_plan(
    world_id: &str,
    turn_id: &str,
    active_relationship_graph: &RelationshipGraphPacket,
    response: &AgentTurnResponse,
) -> Result<ActorAgencyEventPlan> {
    let known_actor_refs = known_actor_refs(active_relationship_graph, response);
    let recorded_at = Utc::now().to_rfc3339();
    let mut goal_records = Vec::new();
    for update in &response.actor_goal_events {
        validate_actor_goal_update(update, &known_actor_refs)?;
        goal_records.push(ActorGoalEventRecord {
            schema_version: ACTOR_GOAL_EVENT_SCHEMA_VERSION.to_owned(),
            world_id: world_id.to_owned(),
            turn_id: turn_id.to_owned(),
            event_id: format!("actor_goal_event:{turn_id}:{:02}", goal_records.len()),
            actor_ref: update.actor_ref.trim().to_owned(),
            goal_id: update.goal_id.trim().to_owned(),
            visibility: update.visibility,
            desire: update.desire.trim().to_owned(),
            fear_or_constraint: update.fear_or_constraint.trim().to_owned(),
            current_leverage: trimmed_vec(&update.current_leverage),
            pressure_refs: trimmed_vec(&update.pressure_refs),
            evidence_refs: trimmed_vec(&update.evidence_refs),
            retired: update.retired,
            recorded_at: recorded_at.clone(),
        });
    }
    let mut move_records = Vec::new();
    for update in &response.actor_move_events {
        validate_actor_move_update(update, &known_actor_refs)?;
        move_records.push(ActorMoveEventRecord {
            schema_version: ACTOR_MOVE_EVENT_SCHEMA_VERSION.to_owned(),
            world_id: world_id.to_owned(),
            turn_id: turn_id.to_owned(),
            event_id: format!("actor_move_event:{turn_id}:{:02}", move_records.len()),
            actor_ref: update.actor_ref.trim().to_owned(),
            move_id: update.move_id.trim().to_owned(),
            visibility: update.visibility,
            action_summary: update.action_summary.trim().to_owned(),
            produced_pressure_refs: trimmed_vec(&update.produced_pressure_refs),
            relationship_refs: trimmed_vec(&update.relationship_refs),
            evidence_refs: trimmed_vec(&update.evidence_refs),
            recorded_at: recorded_at.clone(),
        });
    }
    Ok(ActorAgencyEventPlan {
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        goal_records,
        move_records,
    })
}

pub fn append_actor_agency_event_plan(world_dir: &Path, plan: &ActorAgencyEventPlan) -> Result<()> {
    for record in &plan.goal_records {
        append_jsonl(&world_dir.join(ACTOR_GOAL_EVENTS_FILENAME), record)?;
    }
    for record in &plan.move_records {
        append_jsonl(&world_dir.join(ACTOR_MOVE_EVENTS_FILENAME), record)?;
    }
    Ok(())
}

pub fn rebuild_actor_agency_packet(
    world_dir: &Path,
    base_packet: &ActorAgencyPacket,
) -> Result<ActorAgencyPacket> {
    let goal_records = load_actor_goal_event_records(world_dir)?;
    let move_records = load_actor_move_event_records(world_dir)?;
    let packet = build_actor_agency_from_events(base_packet, &goal_records, &move_records);
    write_json(&world_dir.join(ACTOR_AGENCY_FILENAME), &packet)?;
    Ok(packet)
}

pub fn load_actor_agency_state(
    world_dir: &Path,
    base_packet: ActorAgencyPacket,
) -> Result<ActorAgencyPacket> {
    let path = world_dir.join(ACTOR_AGENCY_FILENAME);
    if path.is_file() {
        return read_json(&path);
    }
    Ok(base_packet)
}

#[must_use]
pub fn build_actor_agency_from_events(
    base_packet: &ActorAgencyPacket,
    goal_records: &[ActorGoalEventRecord],
    move_records: &[ActorMoveEventRecord],
) -> ActorAgencyPacket {
    let mut active_by_goal = base_packet
        .active_goals
        .iter()
        .map(|goal| (goal.goal_id.clone(), goal.clone()))
        .collect::<BTreeMap<_, _>>();
    for record in goal_records
        .iter()
        .filter(|record| record.visibility == ContextVisibility::PlayerVisible)
    {
        if record.retired {
            active_by_goal.remove(&record.goal_id);
        } else {
            active_by_goal.insert(record.goal_id.clone(), actor_goal_from_record(record));
        }
    }
    let mut active_goals = active_by_goal.into_values().collect::<Vec<_>>();
    if active_goals.len() > ACTIVE_ACTOR_GOAL_BUDGET {
        let overflow = active_goals.len() - ACTIVE_ACTOR_GOAL_BUDGET;
        active_goals.drain(0..overflow);
    }

    let mut recent_moves = base_packet.recent_moves.clone();
    recent_moves.extend(
        move_records
            .iter()
            .filter(|record| record.visibility == ContextVisibility::PlayerVisible)
            .map(actor_move_from_record),
    );
    if recent_moves.len() > RECENT_ACTOR_MOVE_BUDGET {
        let overflow = recent_moves.len() - RECENT_ACTOR_MOVE_BUDGET;
        recent_moves.drain(0..overflow);
    }

    ActorAgencyPacket {
        schema_version: ACTOR_AGENCY_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: base_packet.world_id.clone(),
        turn_id: goal_records
            .last()
            .map(|record| record.turn_id.clone())
            .or_else(|| move_records.last().map(|record| record.turn_id.clone()))
            .unwrap_or_else(|| base_packet.turn_id.clone()),
        active_goals,
        recent_moves,
        compiler_policy: ActorAgencyPolicy::default(),
    }
}

pub fn load_actor_goal_event_records(world_dir: &Path) -> Result<Vec<ActorGoalEventRecord>> {
    load_jsonl(
        &world_dir.join(ACTOR_GOAL_EVENTS_FILENAME),
        "ActorGoalEventRecord",
    )
}

pub fn load_actor_move_event_records(world_dir: &Path) -> Result<Vec<ActorMoveEventRecord>> {
    load_jsonl(
        &world_dir.join(ACTOR_MOVE_EVENTS_FILENAME),
        "ActorMoveEventRecord",
    )
}

fn validate_actor_goal_update(
    update: &AgentActorGoalUpdate,
    known_actor_refs: &BTreeSet<String>,
) -> Result<()> {
    validate_known_actor(update.actor_ref.as_str(), known_actor_refs)?;
    validate_required(
        "actor_goal_events",
        &[&update.goal_id, &update.desire, &update.fear_or_constraint],
        &update.evidence_refs,
    )
}

fn validate_actor_move_update(
    update: &AgentActorMoveUpdate,
    known_actor_refs: &BTreeSet<String>,
) -> Result<()> {
    validate_known_actor(update.actor_ref.as_str(), known_actor_refs)?;
    validate_required(
        "actor_move_events",
        &[&update.move_id, &update.action_summary],
        &update.evidence_refs,
    )
}

fn validate_known_actor(actor_ref: &str, known_actor_refs: &BTreeSet<String>) -> Result<()> {
    if actor_ref.trim().is_empty() {
        bail!("actor agency update is missing actor_ref");
    }
    if known_actor_refs.contains(actor_ref.trim()) {
        return Ok(());
    }
    bail!("actor agency update references unknown actor_ref={actor_ref}")
}

fn validate_required(label: &str, fields: &[&String], evidence_refs: &[String]) -> Result<()> {
    if fields.iter().any(|field| field.trim().is_empty()) {
        bail!("{label} contains an empty required field");
    }
    if evidence_refs
        .iter()
        .all(|reference| reference.trim().is_empty())
    {
        bail!("{label} must include evidence_refs");
    }
    Ok(())
}

fn known_actor_refs(
    active_relationship_graph: &RelationshipGraphPacket,
    response: &AgentTurnResponse,
) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    refs.extend(
        active_relationship_graph
            .active_edges
            .iter()
            .flat_map(|edge| [&edge.source_entity_id, &edge.target_entity_id])
            .map(|item| item.trim().to_owned())
            .filter(|item| !item.is_empty() && item != "player"),
    );
    refs.extend(
        response
            .entity_updates
            .iter()
            .map(|update| update.entity_id.trim().to_owned())
            .filter(|item| !item.is_empty() && item.starts_with("char:")),
    );
    refs
}

fn actor_goal_from_record(record: &ActorGoalEventRecord) -> ActorGoal {
    ActorGoal {
        schema_version: ACTOR_GOAL_SCHEMA_VERSION.to_owned(),
        actor_ref: record.actor_ref.clone(),
        goal_id: record.goal_id.clone(),
        visibility: record.visibility,
        desire: record.desire.clone(),
        fear_or_constraint: record.fear_or_constraint.clone(),
        current_leverage: record.current_leverage.clone(),
        pressure_refs: record.pressure_refs.clone(),
        evidence_refs: record.evidence_refs.clone(),
        source_event_id: record.event_id.clone(),
    }
}

fn actor_move_from_record(record: &ActorMoveEventRecord) -> ActorMove {
    ActorMove {
        schema_version: ACTOR_MOVE_SCHEMA_VERSION.to_owned(),
        actor_ref: record.actor_ref.clone(),
        move_id: record.move_id.clone(),
        visibility: record.visibility,
        action_summary: record.action_summary.clone(),
        produced_pressure_refs: record.produced_pressure_refs.clone(),
        relationship_refs: record.relationship_refs.clone(),
        evidence_refs: record.evidence_refs.clone(),
        source_event_id: record.event_id.clone(),
    }
}

fn trimmed_vec(items: &[String]) -> Vec<String> {
    items
        .iter()
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect()
}

fn load_jsonl<T>(path: &Path, label: &str) -> Result<Vec<T>>
where
    T: for<'de> Deserialize<'de>,
{
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    raw.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str::<T>(line).with_context(|| {
                format!(
                    "failed to parse {} line {} as {label}",
                    path.display(),
                    index + 1
                )
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_bridge::{AGENT_TURN_RESPONSE_SCHEMA_VERSION, AgentTurnResponse};
    use crate::models::{NARRATIVE_SCENE_SCHEMA_VERSION, NarrativeScene};
    use crate::relationship_graph::{
        RELATIONSHIP_EDGE_SCHEMA_VERSION, RelationshipEdge, RelationshipGraphPolicy,
    };

    #[test]
    fn materializes_visible_actor_goal_and_move() -> Result<()> {
        let relationship_graph = sample_relationship_graph();
        let response = sample_response();
        let plan = prepare_actor_agency_event_plan(
            "stw_actor",
            "turn_0003",
            &relationship_graph,
            &response,
        )?;
        let packet = build_actor_agency_from_events(
            &ActorAgencyPacket {
                world_id: "stw_actor".to_owned(),
                turn_id: "turn_0003".to_owned(),
                ..ActorAgencyPacket::default()
            },
            &plan.goal_records,
            &plan.move_records,
        );

        assert_eq!(packet.active_goals.len(), 1);
        assert_eq!(packet.recent_moves.len(), 1);
        assert_eq!(packet.active_goals[0].actor_ref, "char:guard");
        Ok(())
    }

    #[test]
    fn rejects_unknown_actor_goal_ref() {
        let relationship_graph = RelationshipGraphPacket::default();
        let response = sample_response();

        let Err(error) = prepare_actor_agency_event_plan(
            "stw_actor",
            "turn_0003",
            &relationship_graph,
            &response,
        ) else {
            panic!("unknown actor should fail");
        };

        assert!(format!("{error:#}").contains("unknown actor_ref=char:guard"));
    }

    fn sample_relationship_graph() -> RelationshipGraphPacket {
        RelationshipGraphPacket {
            schema_version: crate::relationship_graph::RELATIONSHIP_GRAPH_PACKET_SCHEMA_VERSION
                .to_owned(),
            world_id: "stw_actor".to_owned(),
            turn_id: "turn_0003".to_owned(),
            active_edges: vec![RelationshipEdge {
                schema_version: RELATIONSHIP_EDGE_SCHEMA_VERSION.to_owned(),
                edge_id: "rel:char:guard->player:suspicious".to_owned(),
                source_entity_id: "char:guard".to_owned(),
                target_entity_id: "player".to_owned(),
                stance: "suspicious".to_owned(),
                visibility: "player_visible".to_owned(),
                visible_summary: "문지기가 플레이어를 의심한다.".to_owned(),
                source_refs: vec!["visible_scene:guard".to_owned()],
                voice_effects: Vec::new(),
            }],
            compiler_policy: RelationshipGraphPolicy::default(),
        }
    }

    fn sample_response() -> AgentTurnResponse {
        AgentTurnResponse {
            schema_version: AGENT_TURN_RESPONSE_SCHEMA_VERSION.to_owned(),
            world_id: "stw_actor".to_owned(),
            turn_id: "turn_0003".to_owned(),
            resolution_proposal: None,
            scene_director_proposal: None,
            visible_scene: NarrativeScene {
                schema_version: NARRATIVE_SCENE_SCHEMA_VERSION.to_owned(),
                speaker: None,
                text_blocks: vec!["문지기가 손을 들어 길을 막는다.".to_owned()],
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
            actor_goal_events: vec![AgentActorGoalUpdate {
                actor_ref: "char:guard".to_owned(),
                goal_id: "goal:guard:hold_gate".to_owned(),
                visibility: ContextVisibility::PlayerVisible,
                desire: "문을 함부로 열지 않는다.".to_owned(),
                fear_or_constraint: "상관의 문책을 피해야 한다.".to_owned(),
                current_leverage: vec!["문 앞 위치".to_owned()],
                pressure_refs: vec!["pressure:social:gate".to_owned()],
                evidence_refs: vec!["visible_scene:guard".to_owned()],
                retired: false,
            }],
            actor_move_events: vec![AgentActorMoveUpdate {
                actor_ref: "char:guard".to_owned(),
                move_id: "move:guard:block_path".to_owned(),
                visibility: ContextVisibility::PlayerVisible,
                action_summary: "문지기가 길을 막는다.".to_owned(),
                produced_pressure_refs: vec!["pressure:social:gate".to_owned()],
                relationship_refs: vec!["rel:char:guard->player:suspicious".to_owned()],
                evidence_refs: vec!["visible_scene:guard".to_owned()],
            }],
        }
    }
}
