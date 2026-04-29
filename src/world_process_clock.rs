#![allow(clippy::missing_errors_doc)]

use crate::agent_bridge::AgentPrivateAdjudicationContext;
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
) -> WorldProcessEventPlan {
    let recorded_at = Utc::now().to_rfc3339();
    let records = scene_pressure_event_plan
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
    WorldProcessEventPlan {
        world_id: clock.world_id.clone(),
        turn_id: scene_pressure_event_plan.turn_id.clone(),
        records,
    }
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
}
