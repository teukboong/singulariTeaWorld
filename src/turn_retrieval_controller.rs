#![allow(clippy::missing_errors_doc)]

use crate::body_resource::BodyResourcePacket;
use crate::location_graph::LocationGraphPacket;
use crate::models::TurnSnapshot;
use crate::player_intent::PlayerIntentTracePacket;
use crate::plot_thread::{PlotThreadPacket, PlotThreadUrgency};
use crate::relationship_graph::RelationshipGraphPacket;
use crate::scene_pressure::{ScenePressurePacket, ScenePressureUrgency};
use crate::store::{append_jsonl, read_json, write_json};
use crate::world_process_clock::{WorldProcessClockPacket, WorldProcessTempo};
use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const TURN_RETRIEVAL_CONTROLLER_SCHEMA_VERSION: &str = "singulari.turn_retrieval_controller.v1";
pub const TURN_RETRIEVAL_GOAL_SCHEMA_VERSION: &str = "singulari.turn_retrieval_goal.v1";
pub const TURN_RETRIEVAL_ROLE_STANCE_SCHEMA_VERSION: &str =
    "singulari.turn_retrieval_role_stance.v1";
pub const TURN_RETRIEVAL_CUE_SCHEMA_VERSION: &str = "singulari.turn_retrieval_cue.v1";
pub const TURN_RETRIEVAL_CONSTRAINT_SCHEMA_VERSION: &str = "singulari.turn_retrieval_constraint.v1";
pub const TURN_RETRIEVAL_EVENT_SCHEMA_VERSION: &str = "singulari.turn_retrieval_event.v1";
pub const TURN_RETRIEVAL_CONTROLLER_FILENAME: &str = "turn_retrieval_controller.json";
pub const TURN_RETRIEVAL_EVENTS_FILENAME: &str = "turn_retrieval_events.jsonl";

const ACTIVE_GOAL_BUDGET: usize = 8;
const ACTIVE_ROLE_STANCE_BUDGET: usize = 8;
const RETRIEVAL_CUE_BUDGET: usize = 16;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnRetrievalControllerPacket {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub active_goals: Vec<TurnRetrievalGoal>,
    #[serde(default)]
    pub active_role_stance: Vec<TurnRetrievalRoleStance>,
    #[serde(default)]
    pub coherence_constraints: Vec<TurnRetrievalConstraint>,
    #[serde(default)]
    pub correspondence_constraints: Vec<TurnRetrievalConstraint>,
    #[serde(default)]
    pub retrieval_cues: Vec<TurnRetrievalCue>,
    pub retrieval_policy: TurnRetrievalPolicy,
}

impl Default for TurnRetrievalControllerPacket {
    fn default() -> Self {
        Self {
            schema_version: TURN_RETRIEVAL_CONTROLLER_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            turn_id: String::new(),
            active_goals: Vec::new(),
            active_role_stance: Vec::new(),
            coherence_constraints: Vec::new(),
            correspondence_constraints: Vec::new(),
            retrieval_cues: Vec::new(),
            retrieval_policy: TurnRetrievalPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnRetrievalGoal {
    pub schema_version: String,
    pub goal_id: String,
    pub source: TurnRetrievalGoalSource,
    pub summary: String,
    pub priority: TurnRetrievalPriority,
    pub visibility: TurnRetrievalVisibility,
    #[serde(default)]
    pub source_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnRetrievalRoleStance {
    pub schema_version: String,
    pub stance_id: String,
    pub subject_id: String,
    pub summary: String,
    pub visibility: TurnRetrievalVisibility,
    #[serde(default)]
    pub source_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnRetrievalConstraint {
    pub schema_version: String,
    pub constraint_id: String,
    pub summary: String,
    pub visibility: TurnRetrievalVisibility,
    #[serde(default)]
    pub source_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnRetrievalCue {
    pub schema_version: String,
    pub cue_id: String,
    pub cue: String,
    pub reason: TurnRetrievalCueReason,
    #[serde(default)]
    pub target_kinds: Vec<TurnRetrievalTargetKind>,
    pub visibility: TurnRetrievalVisibility,
    #[serde(default)]
    pub source_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnRetrievalPolicy {
    pub source: String,
    pub coherence_weight: u8,
    pub correspondence_weight: u8,
    pub max_capsules: usize,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for TurnRetrievalPolicy {
    fn default() -> Self {
        Self {
            source: "compiled_from_typed_turn_state_v1".to_owned(),
            coherence_weight: 35,
            correspondence_weight: 65,
            max_capsules: 8,
            use_rules: vec![
                "Turn retrieval controls prompt selection only; it is not canon.".to_owned(),
                "Correspondence with recorded visible state beats dramatic coherence.".to_owned(),
                "Role stance must not invent protagonist backstory, temperament, or hidden motive."
                    .to_owned(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnRetrievalEventRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub event_id: String,
    pub event_kind: TurnRetrievalEventKind,
    pub source_ref: String,
    pub summary: String,
    #[serde(default)]
    pub boost_triggers: Vec<String>,
    #[serde(default)]
    pub suppress_triggers: Vec<String>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnRetrievalGoalSource {
    PlayerInput,
    PlotThread,
    ScenePressure,
    BodyResource,
    Location,
    WorldProcess,
    PlayerIntent,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnRetrievalPriority {
    Ambient,
    Normal,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnRetrievalVisibility {
    PlayerVisible,
    InferredVisible,
    AdjudicationOnly,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnRetrievalCueReason {
    CurrentGoalMatch,
    RoleStanceMatch,
    ScenePressureSource,
    ActiveProcessSource,
    SpatialAffordance,
    BodyResourceGate,
    PlayerIntentContinuity,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnRetrievalTargetKind {
    WorldLore,
    RelationshipGraph,
    CharacterTextDesign,
    LocationGraph,
    BodyResource,
    WorldProcess,
    ExtraMemory,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnRetrievalEventKind {
    ControllerCompiled,
    GoalActivated,
    CueActivated,
}

pub struct TurnRetrievalCompileInput<'a> {
    pub world_dir: &'a Path,
    pub world_id: &'a str,
    pub turn_id: &'a str,
    pub snapshot: &'a TurnSnapshot,
    pub player_input: &'a str,
    pub active_plot_threads: &'a PlotThreadPacket,
    pub active_scene_pressure: &'a ScenePressurePacket,
    pub active_relationship_graph: &'a RelationshipGraphPacket,
    pub active_body_resource_state: &'a BodyResourcePacket,
    pub active_location_graph: &'a LocationGraphPacket,
    pub active_world_process_clock: &'a WorldProcessClockPacket,
    pub active_player_intent_trace: &'a PlayerIntentTracePacket,
}

pub fn compile_turn_retrieval_controller(
    input: &TurnRetrievalCompileInput<'_>,
) -> Result<TurnRetrievalControllerPacket> {
    let mut packet = TurnRetrievalControllerPacket {
        schema_version: TURN_RETRIEVAL_CONTROLLER_SCHEMA_VERSION.to_owned(),
        world_id: input.world_id.to_owned(),
        turn_id: input.turn_id.to_owned(),
        active_goals: Vec::new(),
        active_role_stance: Vec::new(),
        coherence_constraints: default_coherence_constraints(input),
        correspondence_constraints: default_correspondence_constraints(input),
        retrieval_cues: Vec::new(),
        retrieval_policy: TurnRetrievalPolicy::default(),
    };

    collect_player_goal(input, &mut packet);
    collect_plot_thread_goals(input, &mut packet);
    collect_scene_pressure_goals(input, &mut packet);
    collect_body_resource_goals(input, &mut packet);
    collect_location_goals(input, &mut packet);
    collect_world_process_goals(input, &mut packet);
    collect_player_intent_goals(input, &mut packet);
    collect_relationship_role_stance(input, &mut packet);
    packet.active_goals.truncate(ACTIVE_GOAL_BUDGET);
    packet
        .active_role_stance
        .truncate(ACTIVE_ROLE_STANCE_BUDGET);
    packet.retrieval_cues.truncate(RETRIEVAL_CUE_BUDGET);

    write_json(
        &input.world_dir.join(TURN_RETRIEVAL_CONTROLLER_FILENAME),
        &packet,
    )?;
    append_turn_retrieval_event(input.world_dir, &controller_event(&packet))?;
    Ok(packet)
}

pub fn load_turn_retrieval_controller_state(
    world_dir: &Path,
    base_packet: TurnRetrievalControllerPacket,
) -> Result<TurnRetrievalControllerPacket> {
    let path = world_dir.join(TURN_RETRIEVAL_CONTROLLER_FILENAME);
    if path.is_file() {
        return read_json(&path);
    }
    Ok(base_packet)
}

fn collect_player_goal(
    input: &TurnRetrievalCompileInput<'_>,
    packet: &mut TurnRetrievalControllerPacket,
) {
    let trimmed = input.player_input.trim();
    if trimmed.is_empty() {
        return;
    }
    push_goal(
        packet,
        TurnRetrievalGoalSource::PlayerInput,
        format!("goal:{}:player_input", input.turn_id),
        trimmed.to_owned(),
        TurnRetrievalPriority::High,
        vec![format!("player_input:{}", input.turn_id)],
    );
    push_cue(
        packet,
        format!("cue:{}:player_input", input.turn_id),
        trimmed,
        TurnRetrievalCueReason::CurrentGoalMatch,
        vec![
            TurnRetrievalTargetKind::WorldLore,
            TurnRetrievalTargetKind::RelationshipGraph,
            TurnRetrievalTargetKind::CharacterTextDesign,
            TurnRetrievalTargetKind::LocationGraph,
        ],
        vec![format!("player_input:{}", input.turn_id)],
    );
}

fn collect_plot_thread_goals(
    input: &TurnRetrievalCompileInput<'_>,
    packet: &mut TurnRetrievalControllerPacket,
) {
    for thread in &input.active_plot_threads.active_visible {
        push_goal(
            packet,
            TurnRetrievalGoalSource::PlotThread,
            format!(
                "goal:{}:{}",
                input.turn_id,
                sanitize_ref(thread.thread_id.as_str())
            ),
            thread.current_question.clone(),
            priority_from_thread(thread.urgency),
            thread.source_refs.clone(),
        );
        push_cue(
            packet,
            format!(
                "cue:{}:{}",
                input.turn_id,
                sanitize_ref(thread.thread_id.as_str())
            ),
            format!("{} {}", thread.title, thread.summary),
            TurnRetrievalCueReason::CurrentGoalMatch,
            vec![
                TurnRetrievalTargetKind::WorldLore,
                TurnRetrievalTargetKind::RelationshipGraph,
                TurnRetrievalTargetKind::ExtraMemory,
            ],
            thread.source_refs.clone(),
        );
    }
}

fn collect_scene_pressure_goals(
    input: &TurnRetrievalCompileInput<'_>,
    packet: &mut TurnRetrievalControllerPacket,
) {
    for pressure in &input.active_scene_pressure.visible_active {
        let summary = pressure
            .choice_affordances
            .first()
            .cloned()
            .unwrap_or_else(|| pressure.prose_effect.paragraph_pressure.clone());
        push_goal(
            packet,
            TurnRetrievalGoalSource::ScenePressure,
            format!(
                "goal:{}:{}",
                input.turn_id,
                sanitize_ref(pressure.pressure_id.as_str())
            ),
            summary,
            priority_from_pressure(pressure.urgency),
            pressure.source_refs.clone(),
        );
        let mut cue = pressure.observable_signals.join(" ");
        if cue.trim().is_empty() {
            cue = pressure.prose_effect.sensory_focus.join(" ");
        }
        if cue.trim().is_empty() {
            cue.clone_from(&pressure.prose_effect.paragraph_pressure);
        }
        push_cue(
            packet,
            format!(
                "cue:{}:{}",
                input.turn_id,
                sanitize_ref(pressure.pressure_id.as_str())
            ),
            cue,
            TurnRetrievalCueReason::ScenePressureSource,
            vec![
                TurnRetrievalTargetKind::WorldLore,
                TurnRetrievalTargetKind::RelationshipGraph,
                TurnRetrievalTargetKind::BodyResource,
            ],
            pressure.source_refs.clone(),
        );
    }
}

fn collect_body_resource_goals(
    input: &TurnRetrievalCompileInput<'_>,
    packet: &mut TurnRetrievalControllerPacket,
) {
    for constraint in &input.active_body_resource_state.body_constraints {
        push_goal(
            packet,
            TurnRetrievalGoalSource::BodyResource,
            format!(
                "goal:{}:{}",
                input.turn_id,
                sanitize_ref(constraint.constraint_id.as_str())
            ),
            constraint.summary.clone(),
            if constraint.severity >= 4 {
                TurnRetrievalPriority::Critical
            } else {
                TurnRetrievalPriority::High
            },
            constraint.source_refs.clone(),
        );
        push_cue(
            packet,
            format!(
                "cue:{}:{}",
                input.turn_id,
                sanitize_ref(constraint.constraint_id.as_str())
            ),
            constraint.summary.clone(),
            TurnRetrievalCueReason::BodyResourceGate,
            vec![
                TurnRetrievalTargetKind::BodyResource,
                TurnRetrievalTargetKind::WorldLore,
            ],
            constraint.source_refs.clone(),
        );
    }
}

fn collect_location_goals(
    input: &TurnRetrievalCompileInput<'_>,
    packet: &mut TurnRetrievalControllerPacket,
) {
    if let Some(location) = &input.active_location_graph.current_location {
        push_cue(
            packet,
            format!("cue:{}:current_location", input.turn_id),
            format!("{} {}", location.name, location.notes.join(" ")),
            TurnRetrievalCueReason::SpatialAffordance,
            vec![
                TurnRetrievalTargetKind::LocationGraph,
                TurnRetrievalTargetKind::WorldLore,
                TurnRetrievalTargetKind::RelationshipGraph,
            ],
            location.source_refs.clone(),
        );
    }
}

fn collect_world_process_goals(
    input: &TurnRetrievalCompileInput<'_>,
    packet: &mut TurnRetrievalControllerPacket,
) {
    for process in &input.active_world_process_clock.visible_processes {
        push_goal(
            packet,
            TurnRetrievalGoalSource::WorldProcess,
            format!(
                "goal:{}:{}",
                input.turn_id,
                sanitize_ref(process.process_id.as_str())
            ),
            process.next_tick_contract.clone(),
            priority_from_process(process.tempo),
            process.source_refs.clone(),
        );
        push_cue(
            packet,
            format!(
                "cue:{}:{}",
                input.turn_id,
                sanitize_ref(process.process_id.as_str())
            ),
            process.summary.clone(),
            TurnRetrievalCueReason::ActiveProcessSource,
            vec![
                TurnRetrievalTargetKind::WorldProcess,
                TurnRetrievalTargetKind::WorldLore,
                TurnRetrievalTargetKind::RelationshipGraph,
            ],
            process.source_refs.clone(),
        );
    }
}

fn collect_player_intent_goals(
    input: &TurnRetrievalCompileInput<'_>,
    packet: &mut TurnRetrievalControllerPacket,
) {
    for intent in &input.active_player_intent_trace.active_intents {
        push_cue(
            packet,
            format!(
                "cue:{}:{}",
                input.turn_id,
                sanitize_ref(intent.event_id.as_str())
            ),
            intent.evidence.clone(),
            TurnRetrievalCueReason::PlayerIntentContinuity,
            vec![
                TurnRetrievalTargetKind::WorldLore,
                TurnRetrievalTargetKind::RelationshipGraph,
                TurnRetrievalTargetKind::CharacterTextDesign,
            ],
            vec![intent.event_id.clone()],
        );
    }
}

fn collect_relationship_role_stance(
    input: &TurnRetrievalCompileInput<'_>,
    packet: &mut TurnRetrievalControllerPacket,
) {
    for edge in &input.active_relationship_graph.active_edges {
        packet.active_role_stance.push(TurnRetrievalRoleStance {
            schema_version: TURN_RETRIEVAL_ROLE_STANCE_SCHEMA_VERSION.to_owned(),
            stance_id: format!(
                "role:{}:{}",
                input.turn_id,
                sanitize_ref(edge.edge_id.as_str())
            ),
            subject_id: edge.source_entity_id.clone(),
            summary: edge.visible_summary.clone(),
            visibility: TurnRetrievalVisibility::PlayerVisible,
            source_refs: edge.source_refs.clone(),
        });
        push_cue(
            packet,
            format!(
                "cue:{}:{}",
                input.turn_id,
                sanitize_ref(edge.edge_id.as_str())
            ),
            format!(
                "{} {} {}",
                edge.source_entity_id, edge.target_entity_id, edge.visible_summary
            ),
            TurnRetrievalCueReason::RoleStanceMatch,
            vec![
                TurnRetrievalTargetKind::RelationshipGraph,
                TurnRetrievalTargetKind::CharacterTextDesign,
            ],
            edge.source_refs.clone(),
        );
    }
}

fn push_goal(
    packet: &mut TurnRetrievalControllerPacket,
    source: TurnRetrievalGoalSource,
    goal_id: String,
    summary: String,
    priority: TurnRetrievalPriority,
    source_refs: Vec<String>,
) {
    if summary.trim().is_empty() {
        return;
    }
    packet.active_goals.push(TurnRetrievalGoal {
        schema_version: TURN_RETRIEVAL_GOAL_SCHEMA_VERSION.to_owned(),
        goal_id,
        source,
        summary,
        priority,
        visibility: TurnRetrievalVisibility::PlayerVisible,
        source_refs,
    });
}

fn push_cue(
    packet: &mut TurnRetrievalControllerPacket,
    cue_id: String,
    cue: impl Into<String>,
    reason: TurnRetrievalCueReason,
    target_kinds: Vec<TurnRetrievalTargetKind>,
    source_refs: Vec<String>,
) {
    let cue = cue.into();
    if cue.trim().is_empty() {
        return;
    }
    packet.retrieval_cues.push(TurnRetrievalCue {
        schema_version: TURN_RETRIEVAL_CUE_SCHEMA_VERSION.to_owned(),
        cue_id,
        cue,
        reason,
        target_kinds,
        visibility: TurnRetrievalVisibility::PlayerVisible,
        source_refs,
    });
}

fn default_coherence_constraints(
    input: &TurnRetrievalCompileInput<'_>,
) -> Vec<TurnRetrievalConstraint> {
    vec![TurnRetrievalConstraint {
        schema_version: TURN_RETRIEVAL_CONSTRAINT_SCHEMA_VERSION.to_owned(),
        constraint_id: format!("coherence:{}:current_scene", input.turn_id),
        summary: "Keep callbacks tied to the current scene objective, pressure, or role stance."
            .to_owned(),
        visibility: TurnRetrievalVisibility::PlayerVisible,
        source_refs: vec![format!("turn_snapshot:{}", input.snapshot.turn_id)],
    }]
}

fn default_correspondence_constraints(
    input: &TurnRetrievalCompileInput<'_>,
) -> Vec<TurnRetrievalConstraint> {
    vec![
        TurnRetrievalConstraint {
            schema_version: TURN_RETRIEVAL_CONSTRAINT_SCHEMA_VERSION.to_owned(),
            constraint_id: format!("correspondence:{}:visibility", input.turn_id),
            summary: "Visible retrieval may use only player-visible or inferred-visible state."
                .to_owned(),
            visibility: TurnRetrievalVisibility::PlayerVisible,
            source_refs: vec![format!("turn_snapshot:{}", input.snapshot.turn_id)],
        },
        TurnRetrievalConstraint {
            schema_version: TURN_RETRIEVAL_CONSTRAINT_SCHEMA_VERSION.to_owned(),
            constraint_id: format!("correspondence:{}:no_backstory", input.turn_id),
            summary: "Do not create protagonist backstory or temperament from role stance."
                .to_owned(),
            visibility: TurnRetrievalVisibility::PlayerVisible,
            source_refs: vec![format!("turn_snapshot:{}", input.snapshot.turn_id)],
        },
    ]
}

fn controller_event(packet: &TurnRetrievalControllerPacket) -> TurnRetrievalEventRecord {
    TurnRetrievalEventRecord {
        schema_version: TURN_RETRIEVAL_EVENT_SCHEMA_VERSION.to_owned(),
        world_id: packet.world_id.clone(),
        turn_id: packet.turn_id.clone(),
        event_id: format!("turn_retrieval_event:{}:compiled", packet.turn_id),
        event_kind: TurnRetrievalEventKind::ControllerCompiled,
        source_ref: format!("turn_retrieval_controller:{}", packet.turn_id),
        summary: format!(
            "compiled {} goals, {} role stances, {} cues",
            packet.active_goals.len(),
            packet.active_role_stance.len(),
            packet.retrieval_cues.len()
        ),
        boost_triggers: packet
            .retrieval_cues
            .iter()
            .take(8)
            .map(|cue| cue.cue.clone())
            .collect(),
        suppress_triggers: vec![
            "unrelated backstory".to_owned(),
            "generic genre exposition".to_owned(),
            "seed leakage".to_owned(),
        ],
        recorded_at: Utc::now().to_rfc3339(),
    }
}

fn append_turn_retrieval_event(world_dir: &Path, event: &TurnRetrievalEventRecord) -> Result<()> {
    let path = world_dir.join(TURN_RETRIEVAL_EVENTS_FILENAME);
    if path.exists() {
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        for (index, line) in raw.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let existing: TurnRetrievalEventRecord =
                serde_json::from_str(line).with_context(|| {
                    format!(
                        "failed to parse {} line {} as TurnRetrievalEventRecord",
                        path.display(),
                        index + 1
                    )
                })?;
            if existing.event_id == event.event_id {
                return Ok(());
            }
        }
    }
    append_jsonl(&path, event)
}

fn priority_from_thread(urgency: PlotThreadUrgency) -> TurnRetrievalPriority {
    match urgency {
        PlotThreadUrgency::Ambient => TurnRetrievalPriority::Ambient,
        PlotThreadUrgency::Soon => TurnRetrievalPriority::Normal,
        PlotThreadUrgency::Immediate => TurnRetrievalPriority::High,
    }
}

fn priority_from_pressure(urgency: ScenePressureUrgency) -> TurnRetrievalPriority {
    match urgency {
        ScenePressureUrgency::Ambient => TurnRetrievalPriority::Ambient,
        ScenePressureUrgency::Soon => TurnRetrievalPriority::Normal,
        ScenePressureUrgency::Immediate => TurnRetrievalPriority::High,
        ScenePressureUrgency::Crisis => TurnRetrievalPriority::Critical,
    }
}

fn priority_from_process(tempo: WorldProcessTempo) -> TurnRetrievalPriority {
    match tempo {
        WorldProcessTempo::Ambient => TurnRetrievalPriority::Ambient,
        WorldProcessTempo::Soon => TurnRetrievalPriority::Normal,
        WorldProcessTempo::Immediate => TurnRetrievalPriority::High,
        WorldProcessTempo::Crisis => TurnRetrievalPriority::Critical,
    }
}

fn sanitize_ref(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body_resource::{BODY_RESOURCE_PACKET_SCHEMA_VERSION, BodyResourcePolicy};
    use crate::location_graph::{LOCATION_GRAPH_PACKET_SCHEMA_VERSION, LocationGraphPolicy};
    use crate::models::{ProtagonistState, TurnSnapshot};
    use crate::player_intent::{PLAYER_INTENT_TRACE_SCHEMA_VERSION, PlayerIntentPolicy};
    use crate::plot_thread::{
        PLOT_THREAD_PACKET_SCHEMA_VERSION, PLOT_THREAD_SCHEMA_VERSION, PlotThread, PlotThreadKind,
        PlotThreadPolicy, PlotThreadStatus,
    };
    use crate::relationship_graph::{
        RELATIONSHIP_EDGE_SCHEMA_VERSION, RELATIONSHIP_GRAPH_PACKET_SCHEMA_VERSION,
        RelationshipEdge, RelationshipGraphPolicy,
    };
    use crate::scene_pressure::{SCENE_PRESSURE_PACKET_SCHEMA_VERSION, ScenePressurePolicy};
    use crate::world_process_clock::{
        WORLD_PROCESS_CLOCK_PACKET_SCHEMA_VERSION, WorldProcessClockPolicy,
    };

    #[test]
    fn active_goal_from_plot_thread_creates_matching_cue_without_backstory() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let input = sample_input(temp.path());
        let packet = compile_turn_retrieval_controller(&input)?;

        assert!(packet.active_goals.iter().any(|goal| {
            goal.source == TurnRetrievalGoalSource::PlotThread && goal.summary.contains("gate")
        }));
        assert!(packet.retrieval_cues.iter().any(|cue| {
            cue.reason == TurnRetrievalCueReason::CurrentGoalMatch && cue.cue.contains("gate")
        }));
        assert!(packet.correspondence_constraints.iter().any(|constraint| {
            constraint
                .summary
                .contains("Do not create protagonist backstory")
        }));
        Ok(())
    }

    #[test]
    fn active_goal_from_selected_choice_creates_current_goal_cue() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let input = sample_input(temp.path());
        let packet = compile_turn_retrieval_controller(&input)?;

        assert!(packet.active_goals.iter().any(|goal| {
            goal.source == TurnRetrievalGoalSource::PlayerInput
                && goal.summary == "gate 앞을 살핀다"
                && goal.priority == TurnRetrievalPriority::High
        }));
        assert!(packet.retrieval_cues.iter().any(|cue| {
            cue.reason == TurnRetrievalCueReason::CurrentGoalMatch
                && cue.cue == "gate 앞을 살핀다"
                && cue.visibility == TurnRetrievalVisibility::PlayerVisible
        }));
        Ok(())
    }

    #[test]
    fn correspondence_policy_beats_coherence_pressure() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let input = sample_input(temp.path());
        let packet = compile_turn_retrieval_controller(&input)?;

        assert!(
            packet.retrieval_policy.correspondence_weight
                > packet.retrieval_policy.coherence_weight
        );
        assert!(packet.retrieval_policy.use_rules.iter().any(|rule| {
            rule.contains("Correspondence with recorded visible state beats dramatic coherence")
        }));
        assert!(packet.correspondence_constraints.iter().any(|constraint| {
            constraint
                .summary
                .contains("Visible retrieval may use only")
        }));
        Ok(())
    }

    #[test]
    fn compile_is_idempotent_for_same_turn_event() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let input = sample_input(temp.path());
        compile_turn_retrieval_controller(&input)?;
        compile_turn_retrieval_controller(&input)?;

        let raw = std::fs::read_to_string(temp.path().join(TURN_RETRIEVAL_EVENTS_FILENAME))?;
        let count = raw.lines().filter(|line| !line.trim().is_empty()).count();
        assert_eq!(count, 1);
        Ok(())
    }

    fn sample_input(world_dir: &Path) -> TurnRetrievalCompileInput<'_> {
        TurnRetrievalCompileInput {
            world_dir,
            world_id: "stw_retrieval",
            turn_id: "turn_0002",
            snapshot: sample_snapshot(),
            player_input: "gate 앞을 살핀다",
            active_plot_threads: sample_plot_threads(),
            active_scene_pressure: sample_scene_pressure(),
            active_relationship_graph: sample_relationships(),
            active_body_resource_state: sample_body_resources(),
            active_location_graph: sample_locations(),
            active_world_process_clock: sample_processes(),
            active_player_intent_trace: sample_intent_trace(),
        }
    }

    fn sample_snapshot() -> &'static TurnSnapshot {
        Box::leak(Box::new(TurnSnapshot {
            schema_version: "singulari.turn_snapshot.v1".to_owned(),
            world_id: "stw_retrieval".to_owned(),
            session_id: "session:test".to_owned(),
            turn_id: "turn_0001".to_owned(),
            phase: "Interlude".to_owned(),
            current_event: None,
            protagonist_state: ProtagonistState {
                location: "gate".to_owned(),
                inventory: Vec::new(),
                body: Vec::new(),
                mind: Vec::new(),
            },
            open_questions: Vec::new(),
            last_choices: Vec::new(),
        }))
    }

    fn sample_plot_threads() -> &'static PlotThreadPacket {
        Box::leak(Box::new(PlotThreadPacket {
            schema_version: PLOT_THREAD_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_retrieval".to_owned(),
            turn_id: "turn_0002".to_owned(),
            active_visible: vec![PlotThread {
                schema_version: PLOT_THREAD_SCHEMA_VERSION.to_owned(),
                thread_id: "thread:gate_access".to_owned(),
                title: "Gate access".to_owned(),
                thread_kind: PlotThreadKind::Access,
                status: PlotThreadStatus::Active,
                urgency: PlotThreadUrgency::Immediate,
                summary: "The gate authority can block passage.".to_owned(),
                current_question: "How can the protagonist pass the gate?".to_owned(),
                source_refs: vec!["canon_event:turn_0001".to_owned()],
                next_scene_hooks: Vec::new(),
            }],
            compiler_policy: PlotThreadPolicy::default(),
        }))
    }

    fn sample_scene_pressure() -> &'static ScenePressurePacket {
        Box::leak(Box::new(ScenePressurePacket {
            schema_version: SCENE_PRESSURE_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_retrieval".to_owned(),
            turn_id: "turn_0002".to_owned(),
            visible_active: Vec::new(),
            hidden_adjudication_only: Vec::new(),
            compiler_policy: ScenePressurePolicy::default(),
        }))
    }

    fn sample_relationships() -> &'static RelationshipGraphPacket {
        Box::leak(Box::new(RelationshipGraphPacket {
            schema_version: RELATIONSHIP_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_retrieval".to_owned(),
            turn_id: "turn_0002".to_owned(),
            active_edges: vec![RelationshipEdge {
                schema_version: RELATIONSHIP_EDGE_SCHEMA_VERSION.to_owned(),
                edge_id: "guard->protagonist:suspicious".to_owned(),
                source_entity_id: "char:guard".to_owned(),
                target_entity_id: "char:protagonist".to_owned(),
                stance: "suspicious".to_owned(),
                visibility: "player_visible".to_owned(),
                visible_summary: "The guard treats the protagonist as an unknown outsider."
                    .to_owned(),
                source_refs: vec!["relationship_graph_event:turn_0001:00".to_owned()],
                voice_effects: Vec::new(),
            }],
            compiler_policy: RelationshipGraphPolicy::default(),
        }))
    }

    fn sample_body_resources() -> &'static BodyResourcePacket {
        Box::leak(Box::new(BodyResourcePacket {
            schema_version: BODY_RESOURCE_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_retrieval".to_owned(),
            turn_id: "turn_0002".to_owned(),
            body_constraints: Vec::new(),
            resources: Vec::new(),
            compiler_policy: BodyResourcePolicy::default(),
        }))
    }

    fn sample_locations() -> &'static LocationGraphPacket {
        Box::leak(Box::new(LocationGraphPacket {
            schema_version: LOCATION_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_retrieval".to_owned(),
            turn_id: "turn_0002".to_owned(),
            current_location: None,
            known_nearby_locations: Vec::new(),
            compiler_policy: LocationGraphPolicy::default(),
        }))
    }

    fn sample_processes() -> &'static WorldProcessClockPacket {
        Box::leak(Box::new(WorldProcessClockPacket {
            schema_version: WORLD_PROCESS_CLOCK_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_retrieval".to_owned(),
            turn_id: "turn_0002".to_owned(),
            visible_processes: Vec::new(),
            adjudication_only_processes: Vec::new(),
            compiler_policy: WorldProcessClockPolicy::default(),
        }))
    }

    fn sample_intent_trace() -> &'static PlayerIntentTracePacket {
        Box::leak(Box::new(PlayerIntentTracePacket {
            schema_version: PLAYER_INTENT_TRACE_SCHEMA_VERSION.to_owned(),
            world_id: "stw_retrieval".to_owned(),
            turn_id: "turn_0002".to_owned(),
            active_intents: Vec::new(),
            compiler_policy: PlayerIntentPolicy::default(),
        }))
    }
}
