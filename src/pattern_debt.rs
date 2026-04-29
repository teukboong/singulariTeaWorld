use crate::agent_bridge::AgentTurnResponse;
use crate::store::{append_jsonl, read_json, write_json};
use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub const PATTERN_DEBT_PACKET_SCHEMA_VERSION: &str = "singulari.pattern_debt_packet.v1";
pub const PATTERN_DEBT_EVENT_SCHEMA_VERSION: &str = "singulari.pattern_debt_event.v1";
pub const PATTERN_DEBT_FILENAME: &str = "pattern_debt.json";
pub const PATTERN_DEBT_EVENTS_FILENAME: &str = "pattern_debt_events.jsonl";

const ACTIVE_PATTERN_DEBT_BUDGET: usize = 8;
const COOLDOWN_THRESHOLD: usize = 3;
const COOLDOWN_TURNS: u8 = 2;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PatternDebtPacket {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub active_patterns: Vec<PatternDebtRecord>,
    pub compiler_policy: PatternDebtPolicy,
}

impl Default for PatternDebtPacket {
    fn default() -> Self {
        Self {
            schema_version: PATTERN_DEBT_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            turn_id: String::new(),
            active_patterns: Vec::new(),
            compiler_policy: PatternDebtPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PatternDebtEventPlan {
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub records: Vec<PatternDebtRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PatternDebtRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub pattern_id: String,
    pub surface: PatternSurface,
    pub recent_count: usize,
    pub cooldown_turns: u8,
    #[serde(default)]
    pub replacement_pressure: Vec<String>,
    #[serde(default)]
    pub evidence_turns: Vec<String>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum PatternSurface {
    ChoiceShape,
    ParagraphClosure,
    SceneBeat,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PatternDebtPolicy {
    pub source: String,
    pub active_pattern_budget: usize,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for PatternDebtPolicy {
    fn default() -> Self {
        Self {
            source: "compiled_from_recent_visible_scene_shapes_v1".to_owned(),
            active_pattern_budget: ACTIVE_PATTERN_DEBT_BUDGET,
            use_rules: vec![
                "Pattern debt is pressure against repetition, not canon truth.".to_owned(),
                "Replacement pressure must change scene mechanics, not inject random world facts."
                    .to_owned(),
                "Cooldown expiration removes pressure without deleting the evidence log."
                    .to_owned(),
            ],
        }
    }
}

/// Prepare pattern-debt observations from the committed visible response.
///
/// # Errors
///
/// Returns an error if prior pattern-debt events cannot be read.
pub fn prepare_pattern_debt_event_plan(
    world_dir: &Path,
    world_id: &str,
    turn_id: &str,
    response: &AgentTurnResponse,
) -> Result<PatternDebtEventPlan> {
    let current = current_observations(world_id, turn_id, response);
    let mut observations = current.clone();
    observations.extend(load_pattern_debt_event_records(world_dir)?);
    let mut records = current;
    records.extend(compile_repeated_patterns(world_id, turn_id, &observations));
    Ok(PatternDebtEventPlan {
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        records,
    })
}

/// Append pattern-debt events.
///
/// # Errors
///
/// Returns an error when the event log cannot be written.
pub fn append_pattern_debt_event_plan(world_dir: &Path, plan: &PatternDebtEventPlan) -> Result<()> {
    for record in &plan.records {
        append_jsonl(&world_dir.join(PATTERN_DEBT_EVENTS_FILENAME), record)?;
    }
    Ok(())
}

/// Rebuild the active pattern-debt packet.
///
/// # Errors
///
/// Returns an error when event records cannot be loaded or the projection file cannot be written.
pub fn rebuild_pattern_debt(
    world_dir: &Path,
    base_packet: &PatternDebtPacket,
) -> Result<PatternDebtPacket> {
    let mut latest_by_pattern = BTreeMap::new();
    for record in load_pattern_debt_event_records(world_dir)? {
        if record.cooldown_turns > 0 {
            latest_by_pattern.insert(record.pattern_id.clone(), record);
        }
    }
    let mut active_patterns = latest_by_pattern.into_values().collect::<Vec<_>>();
    active_patterns.truncate(ACTIVE_PATTERN_DEBT_BUDGET);
    let packet = PatternDebtPacket {
        schema_version: PATTERN_DEBT_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: base_packet.world_id.clone(),
        turn_id: base_packet.turn_id.clone(),
        active_patterns,
        compiler_policy: PatternDebtPolicy::default(),
    };
    write_json(&world_dir.join(PATTERN_DEBT_FILENAME), &packet)?;
    Ok(packet)
}

/// Load materialized pattern debt, or the supplied base packet for new worlds.
///
/// # Errors
///
/// Returns an error when an existing materialized packet cannot be parsed.
pub fn load_pattern_debt_state(
    world_dir: &Path,
    base_packet: PatternDebtPacket,
) -> Result<PatternDebtPacket> {
    let path = world_dir.join(PATTERN_DEBT_FILENAME);
    if path.is_file() {
        return read_json(&path);
    }
    Ok(base_packet)
}

fn load_pattern_debt_event_records(world_dir: &Path) -> Result<Vec<PatternDebtRecord>> {
    let path = world_dir.join(PATTERN_DEBT_EVENTS_FILENAME);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)?;
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str::<PatternDebtRecord>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn current_observations(
    world_id: &str,
    turn_id: &str,
    response: &AgentTurnResponse,
) -> Vec<PatternDebtRecord> {
    let recorded_at = Utc::now().to_rfc3339();
    let mut records = Vec::new();
    for choice in response.next_choices.iter().take(5) {
        records.push(observation(
            world_id,
            turn_id,
            PatternSurface::ChoiceShape,
            format!("choice_shape:{}", normalize_shape(choice.tag.as_str())),
            recorded_at.as_str(),
        ));
    }
    for block in &response.visible_scene.text_blocks {
        if let Some(closure) = paragraph_closure(block) {
            records.push(observation(
                world_id,
                turn_id,
                PatternSurface::ParagraphClosure,
                format!("paragraph_closure:{closure}"),
                recorded_at.as_str(),
            ));
        }
    }
    for note in &response.visible_scene.tone_notes {
        records.push(observation(
            world_id,
            turn_id,
            PatternSurface::SceneBeat,
            format!("scene_beat:{}", normalize_shape(note)),
            recorded_at.as_str(),
        ));
    }
    records
}

fn compile_repeated_patterns(
    world_id: &str,
    turn_id: &str,
    observations: &[PatternDebtRecord],
) -> Vec<PatternDebtRecord> {
    let mut grouped: BTreeMap<String, Vec<&PatternDebtRecord>> = BTreeMap::new();
    for record in observations {
        grouped
            .entry(record.pattern_id.clone())
            .or_default()
            .push(record);
    }
    grouped
        .into_iter()
        .filter_map(|(pattern_id, records)| {
            if records.len() < COOLDOWN_THRESHOLD {
                return None;
            }
            let surface = records[0].surface;
            Some(PatternDebtRecord {
                schema_version: PATTERN_DEBT_EVENT_SCHEMA_VERSION.to_owned(),
                world_id: world_id.to_owned(),
                turn_id: turn_id.to_owned(),
                pattern_id,
                surface,
                recent_count: records.len(),
                cooldown_turns: COOLDOWN_TURNS,
                replacement_pressure: replacement_pressure(surface),
                evidence_turns: records
                    .iter()
                    .rev()
                    .take(COOLDOWN_THRESHOLD)
                    .map(|record| record.turn_id.clone())
                    .collect(),
                recorded_at: Utc::now().to_rfc3339(),
            })
        })
        .collect()
}

fn observation(
    world_id: &str,
    turn_id: &str,
    surface: PatternSurface,
    pattern_id: String,
    recorded_at: &str,
) -> PatternDebtRecord {
    PatternDebtRecord {
        schema_version: PATTERN_DEBT_EVENT_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        pattern_id,
        surface,
        recent_count: 1,
        cooldown_turns: 0,
        replacement_pressure: Vec::new(),
        evidence_turns: vec![turn_id.to_owned()],
        recorded_at: recorded_at.to_owned(),
    }
}

fn normalize_shape(text: &str) -> String {
    text.split_whitespace()
        .take(4)
        .collect::<Vec<_>>()
        .join("_")
        .to_lowercase()
}

fn paragraph_closure(text: &str) -> Option<String> {
    text.split_whitespace()
        .last()
        .map(|word| {
            word.trim_matches(|ch: char| !ch.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|word| !word.is_empty())
}

fn replacement_pressure(surface: PatternSurface) -> Vec<String> {
    match surface {
        PatternSurface::ChoiceShape => vec![
            "change the action cost or permission boundary".to_owned(),
            "offer a materially different affordance".to_owned(),
        ],
        PatternSurface::ParagraphClosure => vec![
            "close the paragraph with consequence or interruption".to_owned(),
            "avoid repeating the same reflective cadence".to_owned(),
        ],
        PatternSurface::SceneBeat => vec![
            "shift to material consequence".to_owned(),
            "introduce social counteraction already justified by visible state".to_owned(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{NarrativeScene, TurnChoice};

    #[test]
    fn repeated_choice_shape_enters_cooldown() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let response = sample_response();
        for turn in ["turn_0001", "turn_0002", "turn_0003"] {
            let plan =
                prepare_pattern_debt_event_plan(temp.path(), "stw_pattern", turn, &response)?;
            append_pattern_debt_event_plan(temp.path(), &plan)?;
        }

        let packet = rebuild_pattern_debt(
            temp.path(),
            &PatternDebtPacket {
                world_id: "stw_pattern".to_owned(),
                turn_id: "turn_0003".to_owned(),
                ..PatternDebtPacket::default()
            },
        )?;

        assert!(
            packet
                .active_patterns
                .iter()
                .any(|pattern| pattern.surface == PatternSurface::ChoiceShape)
        );
        Ok(())
    }

    fn sample_response() -> AgentTurnResponse {
        AgentTurnResponse {
            schema_version: crate::agent_bridge::AGENT_TURN_RESPONSE_SCHEMA_VERSION.to_owned(),
            world_id: "stw_pattern".to_owned(),
            turn_id: "turn_0001".to_owned(),
            resolution_proposal: None,
            scene_director_proposal: None,
            consequence_proposal: None,
            social_exchange_proposal: None,
            visible_scene: NarrativeScene {
                schema_version: crate::models::NARRATIVE_SCENE_SCHEMA_VERSION.to_owned(),
                speaker: None,
                text_blocks: vec!["문이 닫혔다.".to_owned()],
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
            next_choices: vec![TurnChoice {
                slot: 1,
                tag: "관찰".to_owned(),
                intent: "본다".to_owned(),
            }],
            actor_goal_events: Vec::new(),
            actor_move_events: Vec::new(),
        }
    }
}
