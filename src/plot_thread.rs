use crate::models::TurnSnapshot;
use crate::store::append_jsonl;
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

pub const PLOT_THREAD_PACKET_SCHEMA_VERSION: &str = "singulari.plot_thread_packet.v1";
pub const PLOT_THREAD_SCHEMA_VERSION: &str = "singulari.plot_thread.v1";
pub const PLOT_THREAD_AUDIT_SCHEMA_VERSION: &str = "singulari.plot_thread_audit.v1";
pub const PLOT_THREAD_AUDIT_FILENAME: &str = "plot_thread_audit.jsonl";
pub const PLOT_THREAD_EVENT_SCHEMA_VERSION: &str = "singulari.plot_thread_event.v1";
pub const PLOT_THREAD_EVENTS_FILENAME: &str = "plot_thread_events.jsonl";

const ACTIVE_THREAD_BUDGET: usize = 3;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlotThreadPacket {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub active_visible: Vec<PlotThread>,
    pub compiler_policy: PlotThreadPolicy,
}

impl Default for PlotThreadPacket {
    fn default() -> Self {
        Self {
            schema_version: PLOT_THREAD_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            turn_id: String::new(),
            active_visible: Vec::new(),
            compiler_policy: PlotThreadPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlotThread {
    pub schema_version: String,
    pub thread_id: String,
    pub title: String,
    pub thread_kind: PlotThreadKind,
    pub status: PlotThreadStatus,
    pub urgency: PlotThreadUrgency,
    pub summary: String,
    pub current_question: String,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default)]
    pub next_scene_hooks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlotThreadPolicy {
    pub source: String,
    pub active_visible_budget: usize,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlotThreadAuditRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub compiled_at: String,
    pub source: String,
    pub active_visible_count: usize,
    #[serde(default)]
    pub active_thread_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlotThreadEvent {
    pub thread_id: String,
    pub change: PlotThreadChange,
    pub status_after: PlotThreadStatus,
    pub urgency_after: PlotThreadUrgency,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlotThreadChange {
    Advanced,
    Complicated,
    Softened,
    Blocked,
    Resolved,
    Failed,
    Retired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlotThreadEventRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub event_id: String,
    pub thread_id: String,
    pub change: PlotThreadChange,
    pub status_after: PlotThreadStatus,
    pub urgency_after: PlotThreadUrgency,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlotThreadEventPlan {
    pub world_id: String,
    pub turn_id: String,
    pub records: Vec<PlotThreadEventRecord>,
}

impl Default for PlotThreadPolicy {
    fn default() -> Self {
        Self {
            source: "compiled_from_snapshot_v0".to_owned(),
            active_visible_budget: ACTIVE_THREAD_BUDGET,
            use_rules: vec![
                "Use active_plot_threads as unresolved problems, not as quest-log prose."
                    .to_owned(),
                "A thread can shape choices only when the current scene can naturally touch it."
                    .to_owned(),
                "Do not invent hidden plans or genre plotlines from an open thread title."
                    .to_owned(),
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlotThreadKind {
    Access,
    Survival,
    Mystery,
    Relationship,
    Resource,
    Threat,
    Desire,
    MoralCost,
    WorldQuestion,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlotThreadStatus {
    Active,
    Dormant,
    Blocked,
    Resolved,
    Failed,
    Hidden,
    Retired,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlotThreadUrgency {
    Ambient,
    Soon,
    Immediate,
}

#[must_use]
pub fn compile_plot_thread_packet(snapshot: &TurnSnapshot) -> PlotThreadPacket {
    let mut active_visible = snapshot
        .open_questions
        .iter()
        .enumerate()
        .map(|(index, question)| open_question_thread(index, question))
        .collect::<Vec<_>>();
    if let Some(event) = &snapshot.current_event {
        active_visible.insert(
            0,
            PlotThread {
                schema_version: PLOT_THREAD_SCHEMA_VERSION.to_owned(),
                thread_id: format!("thread:current_event:{}", event.event_id),
                title: "현재 사건의 흐름".to_owned(),
                thread_kind: PlotThreadKind::Threat,
                status: PlotThreadStatus::Active,
                urgency: if event.rail_required {
                    PlotThreadUrgency::Immediate
                } else {
                    PlotThreadUrgency::Soon
                },
                summary: format!("current_event {} is at {}", event.event_id, event.progress),
                current_question: "이번 사건이 다음 행동으로 어떻게 변하는가".to_owned(),
                source_refs: vec![format!("current_event:{}", event.event_id)],
                next_scene_hooks: vec![
                    "keep the current event moving from visible evidence".to_owned(),
                ],
            },
        );
    }
    active_visible.truncate(ACTIVE_THREAD_BUDGET);
    PlotThreadPacket {
        schema_version: PLOT_THREAD_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: snapshot.world_id.clone(),
        turn_id: snapshot.turn_id.clone(),
        active_visible,
        compiler_policy: PlotThreadPolicy::default(),
    }
}

pub fn append_plot_thread_audit(world_dir: &Path, packet: &PlotThreadPacket) -> Result<()> {
    let record = PlotThreadAuditRecord {
        schema_version: PLOT_THREAD_AUDIT_SCHEMA_VERSION.to_owned(),
        world_id: packet.world_id.clone(),
        turn_id: packet.turn_id.clone(),
        compiled_at: Utc::now().to_rfc3339(),
        source: packet.compiler_policy.source.clone(),
        active_visible_count: packet.active_visible.len(),
        active_thread_ids: packet
            .active_visible
            .iter()
            .map(|thread| thread.thread_id.clone())
            .collect(),
    };
    append_jsonl(&world_dir.join(PLOT_THREAD_AUDIT_FILENAME), &record)
}

pub fn prepare_plot_thread_event_plan(
    packet: &PlotThreadPacket,
    events: &[PlotThreadEvent],
) -> Result<PlotThreadEventPlan> {
    let known_threads = packet
        .active_visible
        .iter()
        .map(|thread| thread.thread_id.as_str())
        .collect::<BTreeSet<_>>();
    let recorded_at = Utc::now().to_rfc3339();
    let mut records = Vec::with_capacity(events.len());
    for (index, event) in events.iter().enumerate() {
        validate_plot_thread_event(packet, &known_threads, event)
            .with_context(|| format!("invalid plot_thread_events[{index}]"))?;
        records.push(PlotThreadEventRecord {
            schema_version: PLOT_THREAD_EVENT_SCHEMA_VERSION.to_owned(),
            world_id: packet.world_id.clone(),
            turn_id: packet.turn_id.clone(),
            event_id: format!("plot_thread_event:{}:{index:02}", packet.turn_id),
            thread_id: event.thread_id.trim().to_owned(),
            change: event.change,
            status_after: event.status_after,
            urgency_after: event.urgency_after,
            summary: event.summary.trim().to_owned(),
            evidence_refs: event
                .evidence_refs
                .iter()
                .map(|reference| reference.trim().to_owned())
                .collect(),
            recorded_at: recorded_at.clone(),
        });
    }
    Ok(PlotThreadEventPlan {
        world_id: packet.world_id.clone(),
        turn_id: packet.turn_id.clone(),
        records,
    })
}

pub fn append_plot_thread_event_plan(world_dir: &Path, plan: &PlotThreadEventPlan) -> Result<()> {
    for record in &plan.records {
        append_jsonl(&world_dir.join(PLOT_THREAD_EVENTS_FILENAME), record)?;
    }
    Ok(())
}

fn validate_plot_thread_event(
    packet: &PlotThreadPacket,
    known_threads: &BTreeSet<&str>,
    event: &PlotThreadEvent,
) -> Result<()> {
    let thread_id = event.thread_id.trim();
    if thread_id.is_empty() {
        bail!("plot thread event thread_id must not be empty");
    }
    if !known_threads.contains(thread_id) {
        bail!(
            "plot thread event references inactive thread: world_id={}, turn_id={}, thread_id={thread_id}",
            packet.world_id,
            packet.turn_id
        );
    }
    if matches!(
        event.status_after,
        PlotThreadStatus::Hidden | PlotThreadStatus::Dormant
    ) {
        bail!("plot thread event status_after must stay player-visible and actionable");
    }
    if event.summary.trim().is_empty() {
        bail!("plot thread event summary must not be empty");
    }
    if event.evidence_refs.is_empty()
        || event
            .evidence_refs
            .iter()
            .any(|reference| reference.trim().is_empty())
    {
        bail!("plot thread event evidence_refs must contain non-empty visible refs");
    }
    Ok(())
}

fn open_question_thread(index: usize, question: &str) -> PlotThread {
    PlotThread {
        schema_version: PLOT_THREAD_SCHEMA_VERSION.to_owned(),
        thread_id: format!("thread:open_question:{index:02}"),
        title: question.chars().take(48).collect(),
        thread_kind: PlotThreadKind::Mystery,
        status: PlotThreadStatus::Active,
        urgency: PlotThreadUrgency::Soon,
        summary: format!("unresolved player-visible question: {question}"),
        current_question: question.to_owned(),
        source_refs: vec![format!("latest_snapshot.open_questions[{index}]")],
        next_scene_hooks: vec!["preserve the question unless the turn earns an answer".to_owned()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        CurrentEvent, ProtagonistState, TURN_SNAPSHOT_SCHEMA_VERSION, TurnSnapshot,
    };

    #[test]
    fn compiles_open_questions_as_active_threads() {
        let snapshot = TurnSnapshot {
            schema_version: TURN_SNAPSHOT_SCHEMA_VERSION.to_owned(),
            world_id: "stw_threads".to_owned(),
            session_id: "session".to_owned(),
            turn_id: "turn_0002".to_owned(),
            phase: "choice".to_owned(),
            current_event: None,
            protagonist_state: ProtagonistState {
                location: "place:gate".to_owned(),
                inventory: Vec::new(),
                body: Vec::new(),
                mind: Vec::new(),
            },
            open_questions: vec!["who locked the side gate?".to_owned()],
            last_choices: Vec::new(),
        };

        let packet = compile_plot_thread_packet(&snapshot);

        assert_eq!(packet.active_visible.len(), 1);
        assert_eq!(
            packet.active_visible[0].thread_kind,
            PlotThreadKind::Mystery
        );
        assert_eq!(
            packet.active_visible[0].current_question,
            "who locked the side gate?"
        );
    }

    #[test]
    fn current_event_thread_takes_priority() {
        let snapshot = TurnSnapshot {
            schema_version: TURN_SNAPSHOT_SCHEMA_VERSION.to_owned(),
            world_id: "stw_threads".to_owned(),
            session_id: "session".to_owned(),
            turn_id: "turn_0002".to_owned(),
            phase: "choice".to_owned(),
            current_event: Some(CurrentEvent {
                event_id: "evt_gate".to_owned(),
                progress: "guard waiting".to_owned(),
                rail_required: true,
            }),
            protagonist_state: ProtagonistState {
                location: "place:gate".to_owned(),
                inventory: Vec::new(),
                body: Vec::new(),
                mind: Vec::new(),
            },
            open_questions: vec!["who locked the side gate?".to_owned()],
            last_choices: Vec::new(),
        };

        let packet = compile_plot_thread_packet(&snapshot);

        assert_eq!(
            packet.active_visible[0].thread_id,
            "thread:current_event:evt_gate"
        );
        assert_eq!(
            packet.active_visible[0].urgency,
            PlotThreadUrgency::Immediate
        );
    }

    #[test]
    fn prepares_event_records_for_active_visible_threads() -> Result<()> {
        let snapshot = TurnSnapshot {
            schema_version: TURN_SNAPSHOT_SCHEMA_VERSION.to_owned(),
            world_id: "stw_threads".to_owned(),
            session_id: "session".to_owned(),
            turn_id: "turn_0002".to_owned(),
            phase: "choice".to_owned(),
            current_event: None,
            protagonist_state: ProtagonistState {
                location: "place:gate".to_owned(),
                inventory: Vec::new(),
                body: Vec::new(),
                mind: Vec::new(),
            },
            open_questions: vec!["who locked the side gate?".to_owned()],
            last_choices: Vec::new(),
        };
        let packet = compile_plot_thread_packet(&snapshot);

        let plan = prepare_plot_thread_event_plan(
            &packet,
            &[PlotThreadEvent {
                thread_id: "thread:open_question:00".to_owned(),
                change: PlotThreadChange::Advanced,
                status_after: PlotThreadStatus::Active,
                urgency_after: PlotThreadUrgency::Immediate,
                summary: "gate noise gave the question a visible direction".to_owned(),
                evidence_refs: vec!["visible_scene.text_blocks[0]".to_owned()],
            }],
        )?;

        assert_eq!(plan.records.len(), 1);
        assert_eq!(plan.records[0].thread_id, "thread:open_question:00");
        assert_eq!(plan.records[0].event_id, "plot_thread_event:turn_0002:00");
        Ok(())
    }

    #[test]
    fn rejects_events_for_inactive_threads() {
        let packet = PlotThreadPacket {
            schema_version: PLOT_THREAD_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_threads".to_owned(),
            turn_id: "turn_0002".to_owned(),
            active_visible: Vec::new(),
            compiler_policy: PlotThreadPolicy::default(),
        };

        let error = prepare_plot_thread_event_plan(
            &packet,
            &[PlotThreadEvent {
                thread_id: "thread:missing".to_owned(),
                change: PlotThreadChange::Resolved,
                status_after: PlotThreadStatus::Resolved,
                urgency_after: PlotThreadUrgency::Ambient,
                summary: "resolved without a source".to_owned(),
                evidence_refs: vec!["visible_scene.text_blocks[0]".to_owned()],
            }],
        )
        .unwrap_err();

        assert!(error.to_string().contains("invalid plot_thread_events[0]"));
    }
}
