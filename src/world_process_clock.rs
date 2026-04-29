#![allow(clippy::missing_errors_doc)]

use crate::agent_bridge::AgentPrivateAdjudicationContext;
use crate::consequence_spine::{
    ActiveConsequence, ConsequenceKind, ConsequenceSeverity, ConsequenceSpinePacket,
};
use crate::resolution::{ResolutionProposal, ResolutionVisibility};
use crate::scene_pressure::{
    ScenePressure, ScenePressureEventPlan, ScenePressureKind, ScenePressurePacket,
    ScenePressureUrgency,
};
use crate::store::{append_jsonl, read_json, write_json};
use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const WORLD_PROCESS_CLOCK_PACKET_SCHEMA_VERSION: &str =
    "singulari.world_process_clock_packet.v1";
pub const WORLD_PROCESS_SCHEMA_VERSION: &str = "singulari.world_process.v1";
pub const WORLD_PROCESS_EVENT_SCHEMA_VERSION: &str = "singulari.world_process_event.v1";
pub const WORLD_PROCESSES_FILENAME: &str = "world_processes.json";
pub const WORLD_PROCESS_EVENTS_FILENAME: &str = "world_process_events.jsonl";

const PERSISTED_PROCESS_BUDGET: usize = 12;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldProcessClockPacket {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub visible_processes: Vec<WorldProcess>,
    #[serde(default)]
    pub adjudication_only_processes: Vec<WorldProcess>,
    pub compiler_policy: WorldProcessClockPolicy,
}

impl Default for WorldProcessClockPacket {
    fn default() -> Self {
        Self {
            schema_version: WORLD_PROCESS_CLOCK_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            turn_id: String::new(),
            visible_processes: Vec::new(),
            adjudication_only_processes: Vec::new(),
            compiler_policy: WorldProcessClockPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldProcess {
    pub schema_version: String,
    pub process_id: String,
    pub visibility: WorldProcessVisibility,
    pub tempo: WorldProcessTempo,
    pub summary: String,
    pub next_tick_contract: String,
    #[serde(default)]
    pub source_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorldProcessVisibility {
    PlayerVisible,
    AdjudicationOnly,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorldProcessTempo {
    Ambient,
    Soon,
    Immediate,
    Crisis,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldProcessClockPolicy {
    pub source: String,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldProcessEventPlan {
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub records: Vec<WorldProcessEventRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldProcessEventRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub event_id: String,
    pub process_id: String,
    pub visibility: WorldProcessVisibility,
    pub tempo: WorldProcessTempo,
    pub summary: String,
    pub next_tick_contract: String,
    #[serde(default)]
    pub source_refs: Vec<String>,
    pub recorded_at: String,
}

impl Default for WorldProcessClockPolicy {
    fn default() -> Self {
        Self {
            source: "compiled_from_scene_pressure_and_hidden_timers_v1".to_owned(),
            use_rules: vec![
                "Visible processes should advance scene pressure through consequences, not exposition.".to_owned(),
                "Hidden processes may affect adjudication but must not be named in player-visible prose.".to_owned(),
                "A process tick can change pressure, opportunity, cost, or timing; it must not invent unrelated events.".to_owned(),
            ],
        }
    }
}

#[must_use]
pub fn prepare_world_process_event_plan(
    clock: &WorldProcessClockPacket,
    scene_pressure_event_plan: &ScenePressureEventPlan,
    resolution_proposal: Option<&ResolutionProposal>,
) -> WorldProcessEventPlan {
    let recorded_at = Utc::now().to_rfc3339();
    let mut records: Vec<WorldProcessEventRecord> = scene_pressure_event_plan
        .records
        .iter()
        .enumerate()
        .map(|(index, event)| WorldProcessEventRecord {
            schema_version: WORLD_PROCESS_EVENT_SCHEMA_VERSION.to_owned(),
            world_id: clock.world_id.clone(),
            turn_id: scene_pressure_event_plan.turn_id.clone(),
            event_id: format!("world_process_event:{}:{index:02}", scene_pressure_event_plan.turn_id),
            process_id: format!("process:pressure:{}", event.pressure_id),
            visibility: WorldProcessVisibility::PlayerVisible,
            tempo: tempo_from_urgency(event.urgency_after),
            summary: event.summary.clone(),
            next_tick_contract:
                "Process should tick only when player action, time passage, or pressure evidence touches it.".to_owned(),
            source_refs: event.evidence_refs.clone(),
            recorded_at: recorded_at.clone(),
        })
        .collect();
    let offset = records.len();
    if let Some(proposal) = resolution_proposal {
        records.extend(proposal.process_ticks.iter().enumerate().map(|(index, tick)| {
            WorldProcessEventRecord {
                schema_version: WORLD_PROCESS_EVENT_SCHEMA_VERSION.to_owned(),
                world_id: clock.world_id.clone(),
                turn_id: scene_pressure_event_plan.turn_id.clone(),
                event_id: format!(
                    "world_process_event:{}:{:02}",
                    scene_pressure_event_plan.turn_id,
                    offset + index
                ),
                process_id: tick.process_ref.clone(),
                visibility: process_visibility_from_resolution(tick.visibility),
                tempo: process_tempo_for_ref(clock, tick.process_ref.as_str()),
                summary: tick.summary.clone(),
                next_tick_contract:
                    "Process tick was proposed by audited resolution and should continue only from new evidence.".to_owned(),
                source_refs: tick.evidence_refs.clone(),
                recorded_at: recorded_at.clone(),
            }
        }));
    }
    WorldProcessEventPlan {
        world_id: clock.world_id.clone(),
        turn_id: scene_pressure_event_plan.turn_id.clone(),
        records,
    }
}

fn process_visibility_from_resolution(visibility: ResolutionVisibility) -> WorldProcessVisibility {
    match visibility {
        ResolutionVisibility::PlayerVisible => WorldProcessVisibility::PlayerVisible,
        ResolutionVisibility::AdjudicationOnly => WorldProcessVisibility::AdjudicationOnly,
    }
}

fn process_tempo_for_ref(clock: &WorldProcessClockPacket, process_ref: &str) -> WorldProcessTempo {
    clock
        .visible_processes
        .iter()
        .chain(clock.adjudication_only_processes.iter())
        .find(|process| process.process_id == process_ref)
        .map_or(WorldProcessTempo::Ambient, |process| process.tempo)
}

pub fn append_world_process_event_plan(
    world_dir: &Path,
    plan: &WorldProcessEventPlan,
) -> Result<()> {
    for record in &plan.records {
        append_jsonl(&world_dir.join(WORLD_PROCESS_EVENTS_FILENAME), record)?;
    }
    Ok(())
}

pub fn rebuild_world_process_clock(
    world_dir: &Path,
    base_packet: &WorldProcessClockPacket,
) -> Result<WorldProcessClockPacket> {
    let mut visible_processes = base_packet.visible_processes.clone();
    visible_processes.extend(
        load_world_process_event_records(world_dir)?
            .into_iter()
            .filter(|record| record.visibility == WorldProcessVisibility::PlayerVisible)
            .rev()
            .take(PERSISTED_PROCESS_BUDGET)
            .map(world_process_from_event),
    );
    visible_processes.truncate(PERSISTED_PROCESS_BUDGET);
    let packet = WorldProcessClockPacket {
        schema_version: WORLD_PROCESS_CLOCK_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: base_packet.world_id.clone(),
        turn_id: base_packet.turn_id.clone(),
        visible_processes,
        adjudication_only_processes: base_packet.adjudication_only_processes.clone(),
        compiler_policy: WorldProcessClockPolicy {
            source: "materialized_from_world_process_events_v1".to_owned(),
            ..WorldProcessClockPolicy::default()
        },
    };
    write_json(&world_dir.join(WORLD_PROCESSES_FILENAME), &packet)?;
    Ok(packet)
}

pub fn load_world_process_clock_state(
    world_dir: &Path,
    base_packet: WorldProcessClockPacket,
) -> Result<WorldProcessClockPacket> {
    let path = world_dir.join(WORLD_PROCESSES_FILENAME);
    if path.is_file() {
        return read_json(&path);
    }
    Ok(base_packet)
}

#[must_use]
pub fn merge_consequence_world_processes(
    mut packet: WorldProcessClockPacket,
    consequences: &ConsequenceSpinePacket,
) -> WorldProcessClockPacket {
    let mut known = packet
        .visible_processes
        .iter()
        .map(|process| process.process_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    for consequence in &consequences.active {
        if packet.visible_processes.len() >= PERSISTED_PROCESS_BUDGET {
            break;
        }
        if !consequence_should_drive_process(consequence) {
            continue;
        }
        let process = process_from_consequence(consequence);
        if known.insert(process.process_id.clone()) {
            packet.visible_processes.push(process);
        }
    }
    if !consequences.active.is_empty() {
        "materialized_from_world_process_events_and_consequences_v1"
            .clone_into(&mut packet.compiler_policy.source);
    }
    packet
}

fn consequence_should_drive_process(consequence: &ActiveConsequence) -> bool {
    matches!(
        consequence.kind,
        ConsequenceKind::AlarmRaised
            | ConsequenceKind::LocationAccessChanged
            | ConsequenceKind::ProcessAccelerated
            | ConsequenceKind::ProcessDelayed
            | ConsequenceKind::OpportunityOpened
            | ConsequenceKind::OpportunityLost
            | ConsequenceKind::MoralDebt
    ) || matches!(
        consequence.severity,
        ConsequenceSeverity::Major | ConsequenceSeverity::Critical
    )
}

fn process_from_consequence(consequence: &ActiveConsequence) -> WorldProcess {
    WorldProcess {
        schema_version: WORLD_PROCESS_SCHEMA_VERSION.to_owned(),
        process_id: format!("process:consequence:{}", consequence.consequence_id),
        visibility: WorldProcessVisibility::PlayerVisible,
        tempo: tempo_from_consequence_severity(consequence.severity),
        summary: consequence.player_visible_signal.clone(),
        next_tick_contract:
            "Consequence process should tick only when new player action, time passage, or pressure evidence touches it."
                .to_owned(),
        source_refs: vec![consequence.consequence_id.clone()],
    }
}

const fn tempo_from_consequence_severity(severity: ConsequenceSeverity) -> WorldProcessTempo {
    match severity {
        ConsequenceSeverity::Trace | ConsequenceSeverity::Minor => WorldProcessTempo::Ambient,
        ConsequenceSeverity::Moderate => WorldProcessTempo::Soon,
        ConsequenceSeverity::Major => WorldProcessTempo::Immediate,
        ConsequenceSeverity::Critical => WorldProcessTempo::Crisis,
    }
}

fn load_world_process_event_records(world_dir: &Path) -> Result<Vec<WorldProcessEventRecord>> {
    let path = world_dir.join(WORLD_PROCESS_EVENTS_FILENAME);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)?;
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str::<WorldProcessEventRecord>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn world_process_from_event(record: WorldProcessEventRecord) -> WorldProcess {
    WorldProcess {
        schema_version: WORLD_PROCESS_SCHEMA_VERSION.to_owned(),
        process_id: record.process_id,
        visibility: record.visibility,
        tempo: record.tempo,
        summary: record.summary,
        next_tick_contract: record.next_tick_contract,
        source_refs: record.source_refs,
    }
}

#[must_use]
pub fn compile_world_process_clock_packet(
    world_id: &str,
    turn_id: &str,
    scene_pressure: &ScenePressurePacket,
    private_context: &AgentPrivateAdjudicationContext,
) -> WorldProcessClockPacket {
    WorldProcessClockPacket {
        schema_version: WORLD_PROCESS_CLOCK_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        visible_processes: scene_pressure
            .visible_active
            .iter()
            .filter(|pressure| process_relevant_pressure(pressure))
            .enumerate()
            .map(|(index, pressure)| process_from_pressure(index, pressure))
            .collect(),
        adjudication_only_processes: private_context
            .hidden_timers
            .iter()
            .enumerate()
            .map(|(index, timer)| WorldProcess {
                schema_version: WORLD_PROCESS_SCHEMA_VERSION.to_owned(),
                process_id: format!("process:hidden_timer:{index:02}"),
                visibility: WorldProcessVisibility::AdjudicationOnly,
                tempo: tempo_from_remaining_turns(timer.remaining_turns),
                summary: timer.effect.clone(),
                next_tick_contract:
                    "남은 턴을 줄이고 reveal_conditions가 충족될 때만 visible 사실로 승격한다."
                        .to_owned(),
                source_refs: vec![timer.timer_id.clone()],
            })
            .collect(),
        compiler_policy: WorldProcessClockPolicy::default(),
    }
}

fn process_relevant_pressure(pressure: &ScenePressure) -> bool {
    matches!(
        pressure.kind,
        ScenePressureKind::TimePressure
            | ScenePressureKind::Threat
            | ScenePressureKind::Environment
            | ScenePressureKind::Resource
            | ScenePressureKind::SocialPermission
            | ScenePressureKind::Desire
            | ScenePressureKind::MoralCost
    )
}

fn process_from_pressure(index: usize, pressure: &ScenePressure) -> WorldProcess {
    WorldProcess {
        schema_version: WORLD_PROCESS_SCHEMA_VERSION.to_owned(),
        process_id: format!("process:visible_pressure:{index:02}"),
        visibility: WorldProcessVisibility::PlayerVisible,
        tempo: tempo_from_urgency(pressure.urgency),
        summary: pressure.observable_signals.join(" / "),
        next_tick_contract: format!(
            "{} pressure must either intensify, soften, redirect, or resolve according to the next player action.",
            pressure.pressure_id
        ),
        source_refs: vec![pressure.pressure_id.clone()],
    }
}

fn tempo_from_urgency(urgency: ScenePressureUrgency) -> WorldProcessTempo {
    match urgency {
        ScenePressureUrgency::Ambient => WorldProcessTempo::Ambient,
        ScenePressureUrgency::Soon => WorldProcessTempo::Soon,
        ScenePressureUrgency::Immediate => WorldProcessTempo::Immediate,
        ScenePressureUrgency::Crisis => WorldProcessTempo::Crisis,
    }
}

fn tempo_from_remaining_turns(remaining_turns: u32) -> WorldProcessTempo {
    match remaining_turns {
        0 | 1 => WorldProcessTempo::Crisis,
        2 => WorldProcessTempo::Immediate,
        3..=4 => WorldProcessTempo::Soon,
        _ => WorldProcessTempo::Ambient,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_bridge::AgentHiddenTimer;
    use crate::resolution::{
        ActionAmbiguity, ActionInputKind, ActionIntent, NarrativeBrief, ProcessTickCause,
        ProcessTickProposal, RESOLUTION_PROPOSAL_SCHEMA_VERSION, ResolutionOutcome,
        ResolutionOutcomeKind, ResolutionProposal, ResolutionVisibility,
    };
    use crate::scene_pressure::{
        SCENE_PRESSURE_PACKET_SCHEMA_VERSION, SCENE_PRESSURE_SCHEMA_VERSION, ScenePressurePolicy,
        ScenePressureProseEffect, ScenePressureVisibility,
    };

    #[test]
    fn separates_visible_processes_from_hidden_timers() {
        let packet = compile_world_process_clock_packet(
            "stw_clock",
            "turn_0005",
            &ScenePressurePacket {
                schema_version: SCENE_PRESSURE_PACKET_SCHEMA_VERSION.to_owned(),
                world_id: "stw_clock".to_owned(),
                turn_id: "turn_0005".to_owned(),
                visible_active: vec![ScenePressure {
                    schema_version: SCENE_PRESSURE_SCHEMA_VERSION.to_owned(),
                    pressure_id: "pressure:time:door".to_owned(),
                    kind: ScenePressureKind::TimePressure,
                    visibility: ScenePressureVisibility::PlayerVisible,
                    intensity: 3,
                    urgency: ScenePressureUrgency::Immediate,
                    source_refs: vec!["turn:0004".to_owned()],
                    observable_signals: vec!["문틈의 빛이 줄어든다".to_owned()],
                    choice_affordances: Vec::new(),
                    prose_effect: ScenePressureProseEffect {
                        paragraph_pressure: "tight".to_owned(),
                        sensory_focus: Vec::new(),
                        dialogue_style: "cut".to_owned(),
                    },
                }],
                hidden_adjudication_only: Vec::new(),
                compiler_policy: ScenePressurePolicy::default(),
            },
            &AgentPrivateAdjudicationContext {
                hidden_timers: vec![AgentHiddenTimer {
                    timer_id: "timer:hidden:01".to_owned(),
                    kind: "reveal".to_owned(),
                    remaining_turns: 1,
                    effect: "비밀 문양이 반응한다".to_owned(),
                }],
                unrevealed_constraints: Vec::new(),
                plausibility_gates: Vec::new(),
            },
        );

        assert_eq!(packet.visible_processes.len(), 1);
        assert_eq!(packet.adjudication_only_processes.len(), 1);
        assert_eq!(
            packet.visible_processes[0].visibility,
            WorldProcessVisibility::PlayerVisible
        );
        assert_eq!(
            packet.adjudication_only_processes[0].visibility,
            WorldProcessVisibility::AdjudicationOnly
        );
    }

    #[test]
    fn audited_resolution_process_ticks_enter_event_plan() {
        let clock = WorldProcessClockPacket {
            schema_version: WORLD_PROCESS_CLOCK_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_clock".to_owned(),
            turn_id: "turn_0005".to_owned(),
            visible_processes: vec![WorldProcess {
                schema_version: WORLD_PROCESS_SCHEMA_VERSION.to_owned(),
                process_id: "process:visible_pressure:00".to_owned(),
                visibility: WorldProcessVisibility::PlayerVisible,
                tempo: WorldProcessTempo::Immediate,
                summary: "문이 닫히고 있다.".to_owned(),
                next_tick_contract: "시간이 지나면 닫힌다.".to_owned(),
                source_refs: vec!["pressure:time:door".to_owned()],
            }],
            adjudication_only_processes: Vec::new(),
            compiler_policy: WorldProcessClockPolicy::default(),
        };
        let proposal = ResolutionProposal {
            schema_version: RESOLUTION_PROPOSAL_SCHEMA_VERSION.to_owned(),
            world_id: "stw_clock".to_owned(),
            turn_id: "turn_0005".to_owned(),
            interpreted_intent: ActionIntent {
                input_kind: ActionInputKind::PresentedChoice,
                summary: "문 쪽으로 움직인다.".to_owned(),
                target_refs: vec!["process:visible_pressure:00".to_owned()],
                pressure_refs: Vec::new(),
                evidence_refs: vec!["current_turn".to_owned()],
                ambiguity: ActionAmbiguity::Clear,
            },
            outcome: ResolutionOutcome {
                kind: ResolutionOutcomeKind::Delayed,
                summary: "문이 더 닫힌다.".to_owned(),
                evidence_refs: vec!["process:visible_pressure:00".to_owned()],
            },
            gate_results: Vec::new(),
            proposed_effects: Vec::new(),
            process_ticks: vec![ProcessTickProposal {
                process_ref: "process:visible_pressure:00".to_owned(),
                cause: ProcessTickCause::PlayerActionTouchedProcess,
                visibility: ResolutionVisibility::PlayerVisible,
                summary: "문이 조금 더 닫힌다.".to_owned(),
                evidence_refs: vec!["process:visible_pressure:00".to_owned()],
            }],
            narrative_brief: NarrativeBrief {
                visible_summary: "문틈이 좁아진다.".to_owned(),
                required_beats: Vec::new(),
                forbidden_visible_details: Vec::new(),
            },
            next_choice_plan: Vec::new(),
        };
        let plan = prepare_world_process_event_plan(
            &clock,
            &ScenePressureEventPlan {
                world_id: "stw_clock".to_owned(),
                turn_id: "turn_0005".to_owned(),
                records: Vec::new(),
            },
            Some(&proposal),
        );

        assert_eq!(plan.records.len(), 1);
        assert_eq!(plan.records[0].process_id, "process:visible_pressure:00");
        assert_eq!(plan.records[0].tempo, WorldProcessTempo::Immediate);
    }

    #[test]
    fn consequence_creates_visible_world_process() {
        let packet = merge_consequence_world_processes(
            WorldProcessClockPacket {
                world_id: "stw_clock".to_owned(),
                turn_id: "turn_0005".to_owned(),
                ..WorldProcessClockPacket::default()
            },
            &ConsequenceSpinePacket {
                world_id: "stw_clock".to_owned(),
                turn_id: "turn_0005".to_owned(),
                active: vec![ActiveConsequence {
                    schema_version: crate::consequence_spine::CONSEQUENCE_SCHEMA_VERSION.to_owned(),
                    consequence_id: "consequence:turn_0004:alarm".to_owned(),
                    origin_turn_id: "turn_0004".to_owned(),
                    kind: ConsequenceKind::AlarmRaised,
                    scope: crate::consequence_spine::ConsequenceScope::WorldProcess,
                    status: crate::consequence_spine::ConsequenceStatus::Active,
                    severity: ConsequenceSeverity::Critical,
                    summary: "경계가 올라갔다.".to_owned(),
                    player_visible_signal: "수색이 바로 따라붙는다.".to_owned(),
                    source_refs: vec!["turn:0004".to_owned()],
                    linked_entity_refs: Vec::new(),
                    linked_projection_refs: Vec::new(),
                    expected_return: crate::consequence_spine::ConsequenceReturnWindow::NextTurn,
                    decay: crate::consequence_spine::ConsequenceDecay::default(),
                }],
                ..ConsequenceSpinePacket::default()
            },
        );

        assert_eq!(packet.visible_processes.len(), 1);
        assert_eq!(packet.visible_processes[0].tempo, WorldProcessTempo::Crisis);
        assert!(
            packet.visible_processes[0]
                .process_id
                .starts_with("process:consequence:")
        );
    }
}
