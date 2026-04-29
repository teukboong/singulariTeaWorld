use crate::prompt_context::PromptContextPacket;
use crate::resolution::{
    GateStatus, ResolutionOutcomeKind, ResolutionProposal, ResolutionVisibility,
};
use crate::scene_pressure::{
    ScenePressure, ScenePressureKind, ScenePressurePacket, ScenePressureUrgency,
};
use crate::store::{append_jsonl, read_json, write_json};
use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::path::Path;

pub const SCENE_DIRECTOR_PACKET_SCHEMA_VERSION: &str = "singulari.scene_director_packet.v1";
pub const SCENE_DIRECTOR_PROPOSAL_SCHEMA_VERSION: &str = "singulari.scene_director_proposal.v1";
pub const SCENE_DIRECTOR_EVENTS_FILENAME: &str = "scene_director_events.jsonl";
pub const SCENE_ARC_EVENTS_FILENAME: &str = "scene_arc_events.jsonl";
pub const SCENE_DIRECTOR_FILENAME: &str = "scene_director.json";

const DOMINANT_PRESSURE_BUDGET: usize = 3;
const FORBIDDEN_REPETITION_BUDGET: usize = 5;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SceneDirectorPacket {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub current_scene: SceneArc,
    #[serde(default)]
    pub recent_beats: Vec<DramaticBeat>,
    pub pacing_state: PacingState,
    #[serde(default)]
    pub recommended_next_beats: Vec<DramaticBeatRecommendation>,
    #[serde(default)]
    pub forbidden_repetition: Vec<String>,
    #[serde(default)]
    pub tuning_metrics: SceneDirectorTuningMetrics,
    pub paragraph_budget_hint: ParagraphBudgetHint,
    pub compiler_policy: SceneDirectorPolicy,
}

impl Default for SceneDirectorPacket {
    fn default() -> Self {
        Self {
            schema_version: SCENE_DIRECTOR_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            turn_id: String::new(),
            current_scene: SceneArc::default(),
            recent_beats: Vec::new(),
            pacing_state: PacingState::default(),
            recommended_next_beats: Vec::new(),
            forbidden_repetition: Vec::new(),
            tuning_metrics: SceneDirectorTuningMetrics::default(),
            paragraph_budget_hint: ParagraphBudgetHint::default(),
            compiler_policy: SceneDirectorPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SceneArc {
    pub scene_id: String,
    pub scene_question: String,
    pub scene_phase: ScenePhase,
    pub opened_turn_id: String,
    pub current_tension: TensionLevel,
    #[serde(default)]
    pub dominant_pressure_refs: Vec<String>,
    #[serde(default)]
    pub actor_refs: Vec<String>,
    #[serde(default)]
    pub exit_conditions: Vec<SceneExitCondition>,
    #[serde(default)]
    pub unresolved_visible_threads: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SceneDirectorTuningMetrics {
    pub repeated_beat_kind_count: u8,
    pub repeated_choice_shape_count: u8,
    pub high_tension_streak: u8,
    pub turns_in_current_scene: u16,
    pub recent_transition_count: u8,
}

impl Default for SceneArc {
    fn default() -> Self {
        Self {
            scene_id: "scene:active".to_owned(),
            scene_question: "What visible pressure gives this turn a concrete job?".to_owned(),
            scene_phase: ScenePhase::Opening,
            opened_turn_id: String::new(),
            current_tension: TensionLevel::Low,
            dominant_pressure_refs: Vec::new(),
            actor_refs: Vec::new(),
            exit_conditions: Vec::new(),
            unresolved_visible_threads: Vec::new(),
            evidence_refs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DramaticBeat {
    pub beat_id: String,
    pub turn_id: String,
    pub beat_kind: DramaticBeatKind,
    pub turn_function: String,
    pub tension_before: TensionLevel,
    pub tension_after: TensionLevel,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DramaticBeatRecommendation {
    pub beat_kind: DramaticBeatKind,
    pub reason: String,
    #[serde(default)]
    pub pressure_refs: Vec<String>,
    pub advisory_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SceneDirectorProposal {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub scene_id: String,
    pub beat_kind: DramaticBeatKind,
    pub turn_function: String,
    pub tension_before: TensionLevel,
    pub tension_after: TensionLevel,
    pub scene_effect: SceneEffect,
    pub paragraph_strategy: ParagraphStrategy,
    pub choice_strategy: ChoiceStrategy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<SceneTransitionProposal>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SceneDirectorCritique {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub failure_kind: SceneDirectorFailureKind,
    pub message: String,
    #[serde(default)]
    pub rejected_refs: Vec<String>,
    #[serde(default)]
    pub required_changes: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SceneDirectorFailureKind {
    Schema,
    Evidence,
    Repetition,
    Tension,
    Transition,
    Visibility,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParagraphStrategy {
    pub opening_shape: String,
    pub middle_shape: String,
    pub closure_shape: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChoiceStrategy {
    pub must_change_choice_shape: bool,
    #[serde(default)]
    pub avoid_recent_choice_tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SceneTransitionProposal {
    pub from_scene_id: String,
    pub to_scene_question: String,
    pub transition_reason: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SceneDirectorEventPlan {
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub director_records: Vec<SceneDirectorEventRecord>,
    #[serde(default)]
    pub arc_records: Vec<SceneArcEventRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SceneDirectorEventRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub event_id: String,
    pub scene_id: String,
    pub beat_kind: DramaticBeatKind,
    pub turn_function: String,
    pub tension_before: TensionLevel,
    pub tension_after: TensionLevel,
    pub scene_effect: SceneEffect,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SceneArcEventRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub event_id: String,
    pub scene_id: String,
    pub event_kind: SceneArcEventKind,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_scene_question: Option<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SceneArcEventKind {
    SceneOpened,
    SceneQuestionUpdated,
    ExitConditionAdded,
    ExitConditionMet,
    SceneTransitioned,
    SceneClosed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PacingState {
    #[serde(default)]
    pub recent_kind_sequence: Vec<DramaticBeatKind>,
    pub repeated_choice_shape_count: u8,
    pub repeated_closure_shape_count: u8,
    pub repeated_scene_beat_count: u8,
    pub high_tension_turns: u8,
    pub unresolved_probe_turns: u8,
    pub transition_pressure: TransitionPressure,
}

impl Default for PacingState {
    fn default() -> Self {
        Self {
            recent_kind_sequence: Vec::new(),
            repeated_choice_shape_count: 0,
            repeated_closure_shape_count: 0,
            repeated_scene_beat_count: 0,
            high_tension_turns: 0,
            unresolved_probe_turns: 0,
            transition_pressure: TransitionPressure::Low,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SceneExitCondition {
    pub condition: String,
    pub evidence_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParagraphBudgetHint {
    pub recommended_blocks: u8,
    pub opening_strategy: String,
    pub closure_strategy: String,
}

impl Default for ParagraphBudgetHint {
    fn default() -> Self {
        Self {
            recommended_blocks: 3,
            opening_strategy: "Establish the first concrete pressure before expanding context."
                .to_owned(),
            closure_strategy: "Close on the pressure that should guide the next choice.".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SceneDirectorPolicy {
    pub source: String,
    pub mode: String,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for SceneDirectorPolicy {
    fn default() -> Self {
        Self {
            source: "compiled_from_visible_pressure_and_pattern_debt_v1".to_owned(),
            mode: "advisory_only".to_owned(),
            use_rules: vec![
                "SceneDirector is a pacing hint, not a second plot authority.".to_owned(),
                "Use recommended_next_beats to vary turn function, but do not expose beat taxonomy to the player.".to_owned(),
                "Use forbidden_repetition only as pressure against repeated prose, choice shape, or scene beat.".to_owned(),
                "Do not invent canon facts, hidden motives, locations, or outcomes from SceneDirector hints.".to_owned(),
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScenePhase {
    Opening,
    Development,
    Crisis,
    Release,
    TransitionReady,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TensionLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransitionPressure {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DramaticBeatKind {
    Establish,
    Probe,
    Escalate,
    Complicate,
    Reveal,
    Cost,
    ChoicePressure,
    Decompress,
    Transition,
    Cliffhanger,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SceneEffect {
    Established,
    SceneQuestionNarrowed,
    PressureIncreased,
    PressureSoftened,
    CostImposed,
    VisibleFactRevealed,
    ChoiceSurfaceChanged,
    SceneQuestionTransformed,
    NecessaryStall,
}

#[must_use]
pub fn compile_scene_director_packet(
    world_id: &str,
    turn_id: &str,
    scene_pressure: &ScenePressurePacket,
    active_pattern_debt: &Value,
) -> SceneDirectorPacket {
    compile_scene_director_packet_from_input(SceneDirectorCompileInput {
        world_id,
        turn_id,
        scene_pressure,
        active_pattern_debt,
        active_plot_threads: None,
        active_actor_agency: None,
        active_world_process_clock: None,
        active_player_intent_trace: None,
        active_social_exchange: None,
        recent_scene_window: None,
    })
}

#[derive(Debug, Clone, Copy)]
pub struct SceneDirectorCompileInput<'a> {
    pub world_id: &'a str,
    pub turn_id: &'a str,
    pub scene_pressure: &'a ScenePressurePacket,
    pub active_pattern_debt: &'a Value,
    pub active_plot_threads: Option<&'a Value>,
    pub active_actor_agency: Option<&'a Value>,
    pub active_world_process_clock: Option<&'a Value>,
    pub active_player_intent_trace: Option<&'a Value>,
    pub active_social_exchange: Option<&'a Value>,
    pub recent_scene_window: Option<&'a Value>,
}

#[must_use]
pub fn compile_scene_director_packet_from_input(
    input: SceneDirectorCompileInput<'_>,
) -> SceneDirectorPacket {
    let dominant_pressures = dominant_visible_pressures(input.scene_pressure);
    let scene_signals = collect_scene_director_signals(&input);
    let current_tension = tension_from_pressures(&dominant_pressures);
    let repetition = repetition_counts(input.active_pattern_debt);
    let unresolved_probe_turns = scene_signals.unresolved_probe_turns;
    let transition_pressure = transition_pressure(
        current_tension,
        repetition.scene_beat,
        unresolved_probe_turns,
        scene_signals.process_transition_pressure,
    );
    let scene_phase = scene_phase(current_tension, transition_pressure);
    let dominant_pressure_refs = dominant_pressures
        .iter()
        .map(|pressure| pressure.pressure_id.clone())
        .collect::<Vec<_>>();
    let evidence_refs = dominant_pressures
        .iter()
        .flat_map(|pressure| pressure.source_refs.iter().cloned())
        .take(DOMINANT_PRESSURE_BUDGET)
        .collect::<Vec<_>>();
    let scene_question = scene_question(
        dominant_pressures.first().copied(),
        scene_signals.leading_thread_question.as_deref(),
    );
    let scene_id = scene_id_for(input.turn_id, scene_question.as_str());
    let recent_beats = inferred_recent_beats(input.turn_id, current_tension, &dominant_pressures);
    let recommended_next_beats = recommendations(
        current_tension,
        transition_pressure,
        repetition.scene_beat,
        unresolved_probe_turns,
        scene_signals.has_active_actor_leverage,
        scene_signals.has_social_dialogue_pressure,
        scene_signals.has_active_process_pressure,
        &dominant_pressure_refs,
    );
    let forbidden_repetition = forbidden_repetition(input.active_pattern_debt);

    SceneDirectorPacket {
        schema_version: SCENE_DIRECTOR_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: input.world_id.to_owned(),
        turn_id: input.turn_id.to_owned(),
        current_scene: SceneArc {
            scene_id,
            scene_question,
            scene_phase,
            opened_turn_id: input.turn_id.to_owned(),
            current_tension,
            dominant_pressure_refs,
            actor_refs: scene_signals.actor_refs,
            exit_conditions: exit_conditions(scene_phase),
            unresolved_visible_threads: scene_signals.unresolved_visible_threads,
            evidence_refs,
        },
        recent_beats,
        pacing_state: PacingState {
            recent_kind_sequence: Vec::new(),
            repeated_choice_shape_count: repetition.choice_shape,
            repeated_closure_shape_count: repetition.closure_shape,
            repeated_scene_beat_count: repetition.scene_beat,
            high_tension_turns: u8::from(matches!(current_tension, TensionLevel::High)),
            unresolved_probe_turns,
            transition_pressure,
        },
        recommended_next_beats,
        forbidden_repetition,
        tuning_metrics: SceneDirectorTuningMetrics {
            repeated_beat_kind_count: repetition.scene_beat,
            repeated_choice_shape_count: repetition.choice_shape,
            high_tension_streak: u8::from(matches!(current_tension, TensionLevel::High)),
            turns_in_current_scene: 1,
            recent_transition_count: u8::from(matches!(
                transition_pressure,
                TransitionPressure::High
            )),
        },
        paragraph_budget_hint: paragraph_budget_hint(current_tension, transition_pressure),
        compiler_policy: SceneDirectorPolicy::default(),
    }
}

fn scene_id_for(turn_id: &str, scene_question: &str) -> String {
    let mut hasher = DefaultHasher::new();
    scene_question.hash(&mut hasher);
    format!("scene:{turn_id}:{:08x}", hasher.finish() & 0xffff_ffff)
}

#[derive(Debug, Default)]
#[allow(clippy::struct_excessive_bools)]
struct SceneDirectorSignals {
    actor_refs: Vec<String>,
    unresolved_visible_threads: Vec<String>,
    leading_thread_question: Option<String>,
    unresolved_probe_turns: u8,
    process_transition_pressure: bool,
    has_active_actor_leverage: bool,
    has_social_dialogue_pressure: bool,
    has_active_process_pressure: bool,
}

fn collect_scene_director_signals(input: &SceneDirectorCompileInput<'_>) -> SceneDirectorSignals {
    let mut signals = SceneDirectorSignals::default();
    if let Some(plot_threads) = input.active_plot_threads {
        collect_plot_thread_signals(plot_threads, &mut signals);
    }
    if let Some(actor_agency) = input.active_actor_agency {
        collect_actor_agency_signals(actor_agency, &mut signals);
    }
    if let Some(world_process_clock) = input.active_world_process_clock {
        collect_world_process_signals(world_process_clock, &mut signals);
    }
    if let Some(player_intent_trace) = input.active_player_intent_trace {
        collect_player_intent_signals(player_intent_trace, &mut signals);
    }
    if let Some(social_exchange) = input.active_social_exchange {
        collect_social_exchange_signals(social_exchange, &mut signals);
    }
    if let Some(recent_scene_window) = input.recent_scene_window {
        collect_recent_scene_signals(recent_scene_window, &mut signals);
    }
    signals.actor_refs.sort();
    signals.actor_refs.dedup();
    signals.unresolved_visible_threads.sort();
    signals.unresolved_visible_threads.dedup();
    signals
}

fn collect_social_exchange_signals(social_exchange: &Value, signals: &mut SceneDirectorSignals) {
    for stance in social_exchange
        .get("active_stances")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(actor_ref) = stance.get("actor_ref").and_then(Value::as_str) {
            signals.actor_refs.push(actor_ref.to_owned());
        }
        if stance
            .get("intensity")
            .and_then(Value::as_str)
            .is_some_and(|intensity| matches!(intensity, "medium" | "high" | "crisis"))
        {
            signals.has_social_dialogue_pressure = true;
        }
    }
    if social_exchange
        .get("unresolved_asks")
        .and_then(Value::as_array)
        .is_some_and(|asks| !asks.is_empty())
    {
        signals.unresolved_probe_turns = signals.unresolved_probe_turns.saturating_add(1);
    }
    if social_exchange
        .get("active_commitments")
        .and_then(Value::as_array)
        .is_some_and(|commitments| !commitments.is_empty())
    {
        signals.has_social_dialogue_pressure = true;
    }
}

fn collect_plot_thread_signals(plot_threads: &Value, signals: &mut SceneDirectorSignals) {
    let threads = plot_threads
        .get("active_visible")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    for thread in threads {
        let Some(thread_id) = thread.get("thread_id").and_then(Value::as_str) else {
            continue;
        };
        signals
            .unresolved_visible_threads
            .push(thread_id.to_owned());
        if signals.leading_thread_question.is_none() {
            signals.leading_thread_question = thread
                .get("current_question")
                .and_then(Value::as_str)
                .filter(|question| !question.trim().is_empty())
                .map(str::to_owned);
        }
        if thread
            .get("thread_kind")
            .and_then(Value::as_str)
            .is_some_and(|kind| matches!(kind, "mystery" | "access"))
        {
            signals.unresolved_probe_turns = signals.unresolved_probe_turns.saturating_add(1);
        }
    }
}

fn collect_actor_agency_signals(actor_agency: &Value, signals: &mut SceneDirectorSignals) {
    for goal in actor_agency
        .get("active_goals")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(actor_ref) = goal.get("actor_ref").and_then(Value::as_str) {
            signals.actor_refs.push(actor_ref.to_owned());
        }
        if goal
            .get("current_leverage")
            .and_then(Value::as_array)
            .is_some_and(|leverage| !leverage.is_empty())
            || goal
                .get("pressure_refs")
                .and_then(Value::as_array)
                .is_some_and(|refs| !refs.is_empty())
        {
            signals.has_active_actor_leverage = true;
        }
    }
    for actor_move in actor_agency
        .get("recent_moves")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(actor_ref) = actor_move.get("actor_ref").and_then(Value::as_str) {
            signals.actor_refs.push(actor_ref.to_owned());
        }
    }
}

fn collect_world_process_signals(world_process_clock: &Value, signals: &mut SceneDirectorSignals) {
    for process in world_process_clock
        .get("visible_processes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        signals.has_active_process_pressure = true;
        if process
            .get("tempo")
            .and_then(Value::as_str)
            .is_some_and(|tempo| matches!(tempo, "immediate" | "crisis"))
        {
            signals.process_transition_pressure = true;
        }
    }
}

fn collect_player_intent_signals(player_intent_trace: &Value, signals: &mut SceneDirectorSignals) {
    for intent in player_intent_trace
        .get("active_intents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let intent_shape = intent
            .get("intent_shape")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let evidence = intent
            .get("evidence")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if looks_like_probe(intent_shape) || looks_like_probe(evidence) {
            signals.unresolved_probe_turns = signals.unresolved_probe_turns.saturating_add(1);
        }
    }
}

fn collect_recent_scene_signals(recent_scene_window: &Value, signals: &mut SceneDirectorSignals) {
    let Some(items) = recent_scene_window.as_array() else {
        return;
    };
    for item in items.iter().rev().take(3) {
        let text = item.as_str().unwrap_or_default();
        if looks_like_probe(text) {
            signals.unresolved_probe_turns = signals.unresolved_probe_turns.saturating_add(1);
        }
    }
}

fn looks_like_probe(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    normalized.contains("probe")
        || normalized.contains("inspect")
        || normalized.contains("ask")
        || normalized.contains("look")
        || text.contains("본다")
        || text.contains("살핀")
        || text.contains("묻")
        || text.contains("확인")
        || text.contains("단서")
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct RepetitionCounts {
    choice_shape: u8,
    closure_shape: u8,
    scene_beat: u8,
}

fn dominant_visible_pressures(scene_pressure: &ScenePressurePacket) -> Vec<&ScenePressure> {
    let mut pressures = scene_pressure.visible_active.iter().collect::<Vec<_>>();
    pressures.sort_by_key(|pressure| {
        (
            std::cmp::Reverse(urgency_rank(pressure.urgency)),
            std::cmp::Reverse(pressure.intensity),
            pressure.pressure_id.as_str(),
        )
    });
    pressures.truncate(DOMINANT_PRESSURE_BUDGET);
    pressures
}

const fn urgency_rank(urgency: ScenePressureUrgency) -> u8 {
    match urgency {
        ScenePressureUrgency::Ambient => 0,
        ScenePressureUrgency::Soon => 1,
        ScenePressureUrgency::Immediate => 2,
        ScenePressureUrgency::Crisis => 3,
    }
}

fn tension_from_pressures(pressures: &[&ScenePressure]) -> TensionLevel {
    let top = pressures
        .iter()
        .map(|pressure| (urgency_rank(pressure.urgency), pressure.intensity))
        .max()
        .unwrap_or((0, 0));
    if top.0 >= 3 || top.1 >= 4 {
        TensionLevel::High
    } else if top.0 >= 1 || top.1 >= 2 {
        TensionLevel::Medium
    } else {
        TensionLevel::Low
    }
}

fn repetition_counts(active_pattern_debt: &Value) -> RepetitionCounts {
    let mut counts = RepetitionCounts::default();
    for pattern in active_patterns(active_pattern_debt) {
        let Some(surface) = pattern.get("surface").and_then(Value::as_str) else {
            continue;
        };
        let recent_count = u8::try_from(
            pattern
                .get("recent_count")
                .and_then(Value::as_u64)
                .unwrap_or(1),
        )
        .unwrap_or(u8::MAX);
        match surface {
            "choice_shape" => {
                counts.choice_shape = counts.choice_shape.max(recent_count);
            }
            "paragraph_closure" => {
                counts.closure_shape = counts.closure_shape.max(recent_count);
            }
            "scene_beat" => {
                counts.scene_beat = counts.scene_beat.max(recent_count);
            }
            _ => {}
        }
    }
    counts
}

fn active_patterns(active_pattern_debt: &Value) -> impl Iterator<Item = &Value> {
    active_pattern_debt
        .get("active_patterns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn transition_pressure(
    tension: TensionLevel,
    repeated_scene_beat_count: u8,
    unresolved_probe_turns: u8,
    process_transition_pressure: bool,
) -> TransitionPressure {
    if process_transition_pressure
        || unresolved_probe_turns >= 3
        || (repeated_scene_beat_count >= 3 && matches!(tension, TensionLevel::High))
    {
        TransitionPressure::High
    } else if unresolved_probe_turns >= 2
        || repeated_scene_beat_count >= 2
        || matches!(tension, TensionLevel::High)
    {
        TransitionPressure::Medium
    } else {
        TransitionPressure::Low
    }
}

const fn scene_phase(tension: TensionLevel, transition_pressure: TransitionPressure) -> ScenePhase {
    match (tension, transition_pressure) {
        (_, TransitionPressure::High) => ScenePhase::TransitionReady,
        (TensionLevel::High, _) => ScenePhase::Crisis,
        (TensionLevel::Medium, _) => ScenePhase::Development,
        (TensionLevel::Low, _) => ScenePhase::Opening,
    }
}

fn scene_question(
    strongest_pressure: Option<&ScenePressure>,
    leading_thread: Option<&str>,
) -> String {
    if let Some(thread_question) = leading_thread.filter(|question| !question.trim().is_empty()) {
        return thread_question.to_owned();
    }
    match strongest_pressure.map(|pressure| pressure.kind) {
        Some(ScenePressureKind::Threat | ScenePressureKind::TimePressure) => {
            "Can the protagonist act before the visible pressure closes?".to_owned()
        }
        Some(ScenePressureKind::SocialPermission | ScenePressureKind::Desire) => {
            "Can the protagonist shift the social leverage without losing position?".to_owned()
        }
        Some(ScenePressureKind::Knowledge) => {
            "What visible signal changes what the protagonist can safely believe?".to_owned()
        }
        Some(ScenePressureKind::Resource | ScenePressureKind::Body) => {
            "What can the protagonist still do under the current material limits?".to_owned()
        }
        Some(ScenePressureKind::Environment | ScenePressureKind::MoralCost) => {
            "What consequence must the protagonist face before the scene can move?".to_owned()
        }
        None => "What visible pressure gives this turn a concrete job?".to_owned(),
    }
}

fn inferred_recent_beats(
    turn_id: &str,
    current_tension: TensionLevel,
    dominant_pressures: &[&ScenePressure],
) -> Vec<DramaticBeat> {
    let Some(strongest) = dominant_pressures.first() else {
        return vec![DramaticBeat {
            beat_id: format!("beat:{turn_id}:establish"),
            turn_id: turn_id.to_owned(),
            beat_kind: DramaticBeatKind::Establish,
            turn_function: "Orient the player around the first concrete visible pressure."
                .to_owned(),
            tension_before: TensionLevel::Low,
            tension_after: current_tension,
            evidence_refs: Vec::new(),
        }];
    };
    let beat_kind = match strongest.kind {
        ScenePressureKind::Knowledge => DramaticBeatKind::Probe,
        ScenePressureKind::Threat | ScenePressureKind::TimePressure => DramaticBeatKind::Escalate,
        ScenePressureKind::MoralCost => DramaticBeatKind::Cost,
        ScenePressureKind::SocialPermission | ScenePressureKind::Desire => {
            DramaticBeatKind::ChoicePressure
        }
        ScenePressureKind::Body | ScenePressureKind::Resource | ScenePressureKind::Environment => {
            DramaticBeatKind::Complicate
        }
    };
    vec![DramaticBeat {
        beat_id: format!("beat:{turn_id}:inferred"),
        turn_id: turn_id.to_owned(),
        beat_kind,
        turn_function: "Inferred from the strongest visible pressure; use as rhythm context only."
            .to_owned(),
        tension_before: current_tension,
        tension_after: current_tension,
        evidence_refs: strongest.source_refs.clone(),
    }]
}

#[allow(clippy::too_many_lines)]
#[allow(clippy::too_many_arguments)]
fn recommendations(
    tension: TensionLevel,
    transition_pressure: TransitionPressure,
    repeated_scene_beat_count: u8,
    unresolved_probe_turns: u8,
    has_active_actor_leverage: bool,
    has_social_dialogue_pressure: bool,
    has_active_process_pressure: bool,
    pressure_refs: &[String],
) -> Vec<DramaticBeatRecommendation> {
    if matches!(transition_pressure, TransitionPressure::High) {
        return vec![
            recommendation(
                DramaticBeatKind::Transition,
                "Repeated high-pressure beat shape suggests closing or transforming the current scene question.",
                pressure_refs,
            ),
            recommendation(
                DramaticBeatKind::Reveal,
                "If transition is premature, reveal one visible fact that changes the next decision.",
                pressure_refs,
            ),
        ];
    }
    if repeated_scene_beat_count >= 2 {
        return vec![
            recommendation(
                DramaticBeatKind::Complicate,
                "Scene beat repetition is active; change the problem shape instead of raising pressure again.",
                pressure_refs,
            ),
            recommendation(
                DramaticBeatKind::ChoicePressure,
                "Sharpen incompatible options so the next choice is not another inspection loop.",
                pressure_refs,
            ),
        ];
    }
    if unresolved_probe_turns >= 2 {
        return vec![
            recommendation(
                DramaticBeatKind::Reveal,
                "Repeated probing needs a visible answer, failure, or narrowed question.",
                pressure_refs,
            ),
            recommendation(
                DramaticBeatKind::Transition,
                "If the probe has done enough work, move to the next scene question.",
                pressure_refs,
            ),
        ];
    }
    if has_active_actor_leverage {
        return vec![
            recommendation(
                DramaticBeatKind::ChoicePressure,
                "Active actor leverage should sharpen the decision surface rather than stay implicit.",
                pressure_refs,
            ),
            recommendation(
                DramaticBeatKind::Complicate,
                "Let the actor pressure change the problem shape without inventing hidden motive.",
                pressure_refs,
            ),
        ];
    }
    if has_social_dialogue_pressure {
        return vec![
            recommendation(
                DramaticBeatKind::ChoicePressure,
                "Active dialogue stance, commitment, or unresolved ask should reshape the next social option.",
                pressure_refs,
            ),
            recommendation(
                DramaticBeatKind::Complicate,
                "Let the social exchange contract change dialogue posture without exposing hidden motive.",
                pressure_refs,
            ),
        ];
    }
    if has_active_process_pressure {
        return vec![
            recommendation(
                DramaticBeatKind::Escalate,
                "A visible process is moving; advance it only through player action or pressure evidence.",
                pressure_refs,
            ),
            recommendation(
                DramaticBeatKind::Cost,
                "If the process ticks, attach a visible cost or tradeoff.",
                pressure_refs,
            ),
        ];
    }
    match tension {
        TensionLevel::High => vec![
            recommendation(
                DramaticBeatKind::Cost,
                "High tension should produce a visible cost or tradeoff rather than staying static.",
                pressure_refs,
            ),
            recommendation(
                DramaticBeatKind::Cliffhanger,
                "End on a concrete visible change if the scene should continue.",
                pressure_refs,
            ),
        ],
        TensionLevel::Medium => vec![
            recommendation(
                DramaticBeatKind::Probe,
                "Use the next turn to inspect a signal and make the pressure more actionable.",
                pressure_refs,
            ),
            recommendation(
                DramaticBeatKind::Escalate,
                "Raise one visible pressure only if it opens a new decision surface.",
                pressure_refs,
            ),
        ],
        TensionLevel::Low => vec![recommendation(
            DramaticBeatKind::Establish,
            "Establish a concrete visible pressure before expanding prose or lore.",
            pressure_refs,
        )],
    }
}

fn recommendation(
    beat_kind: DramaticBeatKind,
    reason: &str,
    pressure_refs: &[String],
) -> DramaticBeatRecommendation {
    DramaticBeatRecommendation {
        beat_kind,
        reason: reason.to_owned(),
        pressure_refs: pressure_refs.to_vec(),
        advisory_only: true,
    }
}

fn forbidden_repetition(active_pattern_debt: &Value) -> Vec<String> {
    active_patterns(active_pattern_debt)
        .flat_map(|pattern| {
            let pattern_id = pattern
                .get("pattern_id")
                .and_then(Value::as_str)
                .unwrap_or("unknown_pattern");
            let replacement = pattern
                .get("replacement_pressure")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            if replacement.is_empty() {
                vec![format!("Avoid repeating {pattern_id}.")]
            } else {
                replacement
                    .into_iter()
                    .map(|pressure| format!("Avoid repeating {pattern_id}; instead {pressure}."))
                    .collect()
            }
        })
        .take(FORBIDDEN_REPETITION_BUDGET)
        .collect()
}

fn paragraph_budget_hint(
    tension: TensionLevel,
    transition_pressure: TransitionPressure,
) -> ParagraphBudgetHint {
    match (tension, transition_pressure) {
        (_, TransitionPressure::High) => ParagraphBudgetHint {
            recommended_blocks: 4,
            opening_strategy: "Open on the new visible change, not another recap.".to_owned(),
            closure_strategy: "Close by handing off to the changed scene question.".to_owned(),
        },
        (TensionLevel::High, _) => ParagraphBudgetHint {
            recommended_blocks: 5,
            opening_strategy: "Start with immediate sensory pressure or action consequence."
                .to_owned(),
            closure_strategy: "Leave a concrete cost, risk, or incompatible next decision."
                .to_owned(),
        },
        (TensionLevel::Medium, _) => ParagraphBudgetHint {
            recommended_blocks: 4,
            opening_strategy: "Start from the signal the player can act on now.".to_owned(),
            closure_strategy: "End with one actionable uncertainty, not a summary.".to_owned(),
        },
        (TensionLevel::Low, _) => ParagraphBudgetHint {
            recommended_blocks: 3,
            opening_strategy: "Establish the first concrete pressure before expanding context."
                .to_owned(),
            closure_strategy: "Close on the pressure that should guide the next choice.".to_owned(),
        },
    }
}

fn exit_conditions(scene_phase: ScenePhase) -> Vec<SceneExitCondition> {
    if !matches!(scene_phase, ScenePhase::TransitionReady) {
        return Vec::new();
    }
    vec![SceneExitCondition {
        condition: "Answer, transform, or retire the active scene question before adding another same-shaped pressure beat.".to_owned(),
        evidence_ref: "active_scene_director.pacing_state.transition_pressure".to_owned(),
    }]
}

/// Prepare append-only scene director event records from an accepted proposal.
///
/// # Errors
///
/// Returns an error when a supplied proposal has no evidence refs and therefore
/// cannot be safely materialized.
pub fn prepare_scene_director_event_plan(
    packet: &SceneDirectorPacket,
    proposal: Option<&SceneDirectorProposal>,
) -> Result<SceneDirectorEventPlan> {
    let Some(proposal) = proposal else {
        return Ok(SceneDirectorEventPlan {
            world_id: packet.world_id.clone(),
            turn_id: packet.turn_id.clone(),
            director_records: Vec::new(),
            arc_records: Vec::new(),
        });
    };
    if proposal.evidence_refs.is_empty() {
        anyhow::bail!("scene director proposal cannot materialize without evidence refs");
    }
    let recorded_at = Utc::now().to_rfc3339();
    let director_record = SceneDirectorEventRecord {
        schema_version: "singulari.scene_director_event.v1".to_owned(),
        world_id: packet.world_id.clone(),
        turn_id: packet.turn_id.clone(),
        event_id: format!("scene_director_event:{}:00", packet.turn_id),
        scene_id: proposal.scene_id.clone(),
        beat_kind: proposal.beat_kind,
        turn_function: proposal.turn_function.clone(),
        tension_before: proposal.tension_before,
        tension_after: proposal.tension_after,
        scene_effect: proposal.scene_effect,
        evidence_refs: proposal.evidence_refs.clone(),
        recorded_at: recorded_at.clone(),
    };
    let mut arc_records = Vec::new();
    if let Some(transition) = &proposal.transition {
        arc_records.push(SceneArcEventRecord {
            schema_version: "singulari.scene_arc_event.v1".to_owned(),
            world_id: packet.world_id.clone(),
            turn_id: packet.turn_id.clone(),
            event_id: format!("scene_arc_event:{}:00", packet.turn_id),
            scene_id: proposal.scene_id.clone(),
            event_kind: SceneArcEventKind::SceneTransitioned,
            summary: transition.transition_reason.clone(),
            to_scene_question: Some(transition.to_scene_question.clone()),
            evidence_refs: transition.evidence_refs.clone(),
            recorded_at,
        });
    }
    Ok(SceneDirectorEventPlan {
        world_id: packet.world_id.clone(),
        turn_id: packet.turn_id.clone(),
        director_records: vec![director_record],
        arc_records,
    })
}

/// Append scene director event records to the world log.
///
/// # Errors
///
/// Returns an error when a log file cannot be written.
pub fn append_scene_director_event_plan(
    world_dir: &Path,
    plan: &SceneDirectorEventPlan,
) -> Result<()> {
    if plan.director_records.is_empty() && plan.arc_records.is_empty() {
        return Ok(());
    }
    for record in &plan.director_records {
        append_jsonl(
            world_dir.join(SCENE_DIRECTOR_EVENTS_FILENAME).as_path(),
            record,
        )?;
    }
    for record in &plan.arc_records {
        append_jsonl(world_dir.join(SCENE_ARC_EVENTS_FILENAME).as_path(), record)?;
    }
    Ok(())
}

/// Rebuild the compact materialized scene director packet from event logs.
///
/// # Errors
///
/// Returns an error when event logs cannot be read or the materialized packet
/// cannot be written.
pub fn rebuild_scene_director(
    world_dir: &Path,
    base_packet: &SceneDirectorPacket,
) -> Result<SceneDirectorPacket> {
    let mut packet = base_packet.clone();
    let records = load_scene_director_event_records(world_dir)?;
    let arc_records = load_scene_arc_event_records(world_dir)?;
    apply_latest_scene_arc(&mut packet, &arc_records);
    let recent_records = records.iter().rev().take(8).collect::<Vec<_>>();
    packet.recent_beats = recent_records
        .iter()
        .rev()
        .map(|record| DramaticBeat {
            beat_id: record.event_id.clone(),
            turn_id: record.turn_id.clone(),
            beat_kind: record.beat_kind,
            turn_function: record.turn_function.clone(),
            tension_before: record.tension_before,
            tension_after: record.tension_after,
            evidence_refs: record.evidence_refs.clone(),
        })
        .collect();
    packet.pacing_state.recent_kind_sequence = packet
        .recent_beats
        .iter()
        .map(|beat| beat.beat_kind)
        .collect();
    packet.pacing_state.high_tension_turns =
        consecutive_high_tension_count(records.iter().rev().take(8));
    packet.pacing_state.transition_pressure =
        materialized_transition_pressure(&packet.pacing_state, &packet.recent_beats);
    packet.tuning_metrics = SceneDirectorTuningMetrics {
        repeated_beat_kind_count: repeated_last_beat_count(&packet.recent_beats),
        repeated_choice_shape_count: packet.pacing_state.repeated_choice_shape_count,
        high_tension_streak: packet.pacing_state.high_tension_turns,
        turns_in_current_scene: u16::try_from(packet.recent_beats.len()).unwrap_or(u16::MAX),
        recent_transition_count: u8::try_from(
            packet
                .recent_beats
                .iter()
                .filter(|beat| beat.beat_kind == DramaticBeatKind::Transition)
                .count(),
        )
        .unwrap_or(u8::MAX),
    };
    write_json(world_dir.join(SCENE_DIRECTOR_FILENAME).as_path(), &packet)?;
    Ok(packet)
}

/// Load the materialized scene director packet, or return the supplied fallback.
///
/// # Errors
///
/// Returns an error when an existing materialized packet cannot be read.
pub fn load_scene_director_state(
    world_dir: &Path,
    fallback: SceneDirectorPacket,
) -> Result<SceneDirectorPacket> {
    let path = world_dir.join(SCENE_DIRECTOR_FILENAME);
    if path.exists() {
        return read_json(path.as_path())
            .with_context(|| format!("failed to read {}", path.display()));
    }
    Ok(fallback)
}

#[must_use]
pub fn merge_scene_director_history(
    mut fresh: SceneDirectorPacket,
    history: Option<&SceneDirectorPacket>,
) -> SceneDirectorPacket {
    let Some(history) = history else {
        return fresh;
    };
    preserve_scene_identity(&mut fresh, history);
    fresh.recent_beats.clone_from(&history.recent_beats);
    fresh
        .pacing_state
        .recent_kind_sequence
        .clone_from(&history.pacing_state.recent_kind_sequence);
    fresh.pacing_state.high_tension_turns = history.pacing_state.high_tension_turns;
    fresh.pacing_state.transition_pressure = stronger_transition_pressure(
        fresh.pacing_state.transition_pressure,
        history.pacing_state.transition_pressure,
    );
    fresh.tuning_metrics = SceneDirectorTuningMetrics {
        repeated_beat_kind_count: repeated_last_beat_count(&fresh.recent_beats),
        repeated_choice_shape_count: fresh.pacing_state.repeated_choice_shape_count,
        high_tension_streak: fresh.pacing_state.high_tension_turns,
        turns_in_current_scene: u16::try_from(fresh.recent_beats.len().saturating_add(1))
            .unwrap_or(u16::MAX),
        recent_transition_count: u8::try_from(
            fresh
                .recent_beats
                .iter()
                .filter(|beat| beat.beat_kind == DramaticBeatKind::Transition)
                .count(),
        )
        .unwrap_or(u8::MAX),
    };
    fresh
}

fn preserve_scene_identity(fresh: &mut SceneDirectorPacket, history: &SceneDirectorPacket) {
    if history.current_scene.scene_id.trim().is_empty() {
        return;
    }
    fresh
        .current_scene
        .scene_id
        .clone_from(&history.current_scene.scene_id);
    fresh
        .current_scene
        .opened_turn_id
        .clone_from(&history.current_scene.opened_turn_id);
    if history.current_scene.scene_question != SceneArc::default().scene_question {
        fresh
            .current_scene
            .scene_question
            .clone_from(&history.current_scene.scene_question);
    }
}

const fn stronger_transition_pressure(
    left: TransitionPressure,
    right: TransitionPressure,
) -> TransitionPressure {
    match (left, right) {
        (TransitionPressure::High, _) | (_, TransitionPressure::High) => TransitionPressure::High,
        (TransitionPressure::Medium, _) | (_, TransitionPressure::Medium) => {
            TransitionPressure::Medium
        }
        _ => TransitionPressure::Low,
    }
}

fn repeated_last_beat_count(recent_beats: &[DramaticBeat]) -> u8 {
    let count = recent_beats.last().map_or(0, |last| {
        recent_beats
            .iter()
            .rev()
            .take_while(|beat| beat.beat_kind == last.beat_kind)
            .count()
    });
    u8::try_from(count).unwrap_or(u8::MAX)
}

fn load_scene_director_event_records(world_dir: &Path) -> Result<Vec<SceneDirectorEventRecord>> {
    let path = world_dir.join(SCENE_DIRECTOR_EVENTS_FILENAME);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(path.as_path())
        .with_context(|| format!("failed to read {}", path.display()))?;
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<SceneDirectorEventRecord>(line).map_err(Into::into))
        .collect()
}

fn load_scene_arc_event_records(world_dir: &Path) -> Result<Vec<SceneArcEventRecord>> {
    let path = world_dir.join(SCENE_ARC_EVENTS_FILENAME);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(path.as_path())
        .with_context(|| format!("failed to read {}", path.display()))?;
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<SceneArcEventRecord>(line).map_err(Into::into))
        .collect()
}

fn apply_latest_scene_arc(packet: &mut SceneDirectorPacket, records: &[SceneArcEventRecord]) {
    let Some(transition) = records
        .iter()
        .rev()
        .find(|record| record.event_kind == SceneArcEventKind::SceneTransitioned)
    else {
        return;
    };
    let Some(next_question) = transition
        .to_scene_question
        .as_ref()
        .filter(|question| !question.trim().is_empty())
    else {
        return;
    };
    packet.current_scene.scene_id = scene_id_for(&transition.turn_id, next_question);
    packet
        .current_scene
        .scene_question
        .clone_from(next_question);
    packet
        .current_scene
        .opened_turn_id
        .clone_from(&transition.turn_id);
    packet.current_scene.scene_phase = ScenePhase::Opening;
    packet.current_scene.exit_conditions.clear();
    packet
        .current_scene
        .evidence_refs
        .clone_from(&transition.evidence_refs);
}

fn consecutive_high_tension_count<'a>(
    records: impl Iterator<Item = &'a SceneDirectorEventRecord>,
) -> u8 {
    u8::try_from(
        records
            .take_while(|record| matches!(record.tension_after, TensionLevel::High))
            .count()
            .min(usize::from(u8::MAX)),
    )
    .unwrap_or(u8::MAX)
}

fn materialized_transition_pressure(
    pacing_state: &PacingState,
    recent_beats: &[DramaticBeat],
) -> TransitionPressure {
    if pacing_state.high_tension_turns >= 3 {
        return TransitionPressure::High;
    }
    let repeated_last_kind = repeated_last_beat_count(recent_beats);
    if repeated_last_kind >= 3 {
        TransitionPressure::High
    } else if repeated_last_kind >= 2 || pacing_state.high_tension_turns >= 2 {
        TransitionPressure::Medium
    } else {
        pacing_state.transition_pressure
    }
}

/// Audit an optional LLM-authored scene director proposal against visible
/// prompt context.
///
/// The proposal is advisory metadata, but once supplied it must stay grounded:
/// no hidden refs, no unsupported transitions, and no same-shaped high-tension
/// stalls when the active director packet is already asking for movement.
///
/// # Errors
///
/// Returns a structured critique when the proposal cannot be safely committed.
#[allow(clippy::too_many_lines)]
pub fn audit_scene_director_proposal(
    context: &PromptContextPacket,
    proposal: &SceneDirectorProposal,
    resolution_proposal: Option<&ResolutionProposal>,
    next_choices: &[crate::models::TurnChoice],
) -> std::result::Result<(), Box<SceneDirectorCritique>> {
    let director: SceneDirectorPacket = serde_json::from_value(
        context.visible_context.active_scene_director.clone(),
    )
    .map_err(|error| {
        scene_director_critique(
            context,
            SceneDirectorFailureKind::Schema,
            format!("prompt context active_scene_director is invalid: {error}"),
            Vec::new(),
            vec!["Recompile prompt context before auditing scene director proposal.".to_owned()],
        )
    })?;
    if proposal.world_id != context.world_id || proposal.turn_id != context.turn_id {
        return Err(scene_director_critique(
            context,
            SceneDirectorFailureKind::Schema,
            "scene director proposal world_id/turn_id does not match prompt context",
            vec![proposal.world_id.clone(), proposal.turn_id.clone()],
            vec!["Use the exact world_id and turn_id from prompt_context.".to_owned()],
        ));
    }
    if proposal.scene_id != director.current_scene.scene_id && proposal.transition.is_none() {
        return Err(scene_director_critique(
            context,
            SceneDirectorFailureKind::Schema,
            "scene director proposal scene_id does not match active scene",
            vec![proposal.scene_id.clone()],
            vec![
                "Use active_scene_director.current_scene.scene_id or provide a transition."
                    .to_owned(),
            ],
        ));
    }
    if proposal.evidence_refs.is_empty() {
        return Err(scene_director_critique(
            context,
            SceneDirectorFailureKind::Evidence,
            "scene director proposal is missing evidence refs",
            Vec::new(),
            vec!["Add player-visible evidence_refs from prompt_context.".to_owned()],
        ));
    }
    let visible_refs = collect_visible_strings(
        &serde_json::to_value(&context.visible_context).map_err(|error| {
            scene_director_critique(
                context,
                SceneDirectorFailureKind::Schema,
                format!("visible context cannot be serialized: {error}"),
                Vec::new(),
                Vec::new(),
            )
        })?,
    );
    let rejected_refs = proposal
        .evidence_refs
        .iter()
        .filter(|evidence_ref| !visible_refs.contains(evidence_ref.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !rejected_refs.is_empty() {
        return Err(scene_director_critique(
            context,
            SceneDirectorFailureKind::Evidence,
            "scene director proposal references evidence not present in visible prompt context",
            rejected_refs,
            vec!["Use exact player-visible refs or strings physically present in prompt_context.visible_context.".to_owned()],
        ));
    }
    if matches!(
        director.pacing_state.transition_pressure,
        TransitionPressure::High
    ) && !matches!(
        proposal.beat_kind,
        DramaticBeatKind::Transition
            | DramaticBeatKind::Reveal
            | DramaticBeatKind::Cost
            | DramaticBeatKind::Decompress
    ) && proposal.scene_effect != SceneEffect::NecessaryStall
    {
        return Err(scene_director_critique(
            context,
            SceneDirectorFailureKind::Repetition,
            "active scene director requires movement, but proposal keeps a same-shaped beat",
            vec![format!("{:?}", proposal.beat_kind)],
            vec![
                "Use transition, reveal, cost, decompression, or mark a justified necessary_stall."
                    .to_owned(),
            ],
        ));
    }
    if director.pacing_state.high_tension_turns >= 3
        && matches!(
            (proposal.tension_before, proposal.tension_after),
            (TensionLevel::High, TensionLevel::High)
        )
        && !matches!(
            proposal.beat_kind,
            DramaticBeatKind::Cost
                | DramaticBeatKind::Reveal
                | DramaticBeatKind::Decompress
                | DramaticBeatKind::Transition
        )
    {
        return Err(scene_director_critique(
            context,
            SceneDirectorFailureKind::Tension,
            "high tension remains unchanged without cost, reveal, decompression, or transition",
            vec![format!("{:?}", proposal.beat_kind)],
            vec![
                "Pay off high tension with a visible cost/reveal, lower it, or transition the scene."
                    .to_owned(),
            ],
        ));
    }
    if let Some(transition) = &proposal.transition {
        if proposal.scene_id != director.current_scene.scene_id
            || transition.from_scene_id != director.current_scene.scene_id
        {
            return Err(scene_director_critique(
                context,
                SceneDirectorFailureKind::Schema,
                "transition proposal does not start from the active scene",
                vec![
                    proposal.scene_id.clone(),
                    transition.from_scene_id.clone(),
                    director.current_scene.scene_id,
                ],
                vec![
                    "Use active_scene_director.current_scene.scene_id for proposal.scene_id and transition.from_scene_id."
                        .to_owned(),
                ],
            ));
        }
        if director.current_scene.exit_conditions.is_empty()
            && proposal.scene_effect != SceneEffect::SceneQuestionTransformed
        {
            return Err(scene_director_critique(
                context,
                SceneDirectorFailureKind::Transition,
                "transition proposal lacks an active exit condition or transformed scene question",
                transition.evidence_refs.clone(),
                vec![
                    "Meet/transform an exit condition before proposing a scene transition."
                        .to_owned(),
                ],
            ));
        }
        if transition.evidence_refs.is_empty() {
            return Err(scene_director_critique(
                context,
                SceneDirectorFailureKind::Evidence,
                "transition proposal is missing evidence refs",
                Vec::new(),
                vec!["Add player-visible evidence refs for the transition.".to_owned()],
            ));
        }
    }
    audit_scene_director_resolution_coupling(context, proposal, resolution_proposal)?;
    audit_scene_director_choice_strategy(context, proposal, next_choices)?;
    if contains_hidden_context_text(context, proposal) {
        return Err(scene_director_critique(
            context,
            SceneDirectorFailureKind::Visibility,
            "scene director proposal includes hidden/adjudication-only text",
            Vec::new(),
            vec!["Rewrite scene director proposal using player-visible context only.".to_owned()],
        ));
    }
    Ok(())
}

fn audit_scene_director_resolution_coupling(
    context: &PromptContextPacket,
    proposal: &SceneDirectorProposal,
    resolution_proposal: Option<&ResolutionProposal>,
) -> std::result::Result<(), Box<SceneDirectorCritique>> {
    let Some(resolution) = resolution_proposal else {
        if matches!(
            proposal.beat_kind,
            DramaticBeatKind::Cost | DramaticBeatKind::Reveal
        ) {
            return Err(scene_director_critique(
                context,
                SceneDirectorFailureKind::Evidence,
                "cost/reveal scene director beats require a resolution proposal",
                Vec::new(),
                vec!["Add a resolution_proposal that grounds the cost or reveal.".to_owned()],
            ));
        }
        return Ok(());
    };
    if proposal.beat_kind == DramaticBeatKind::Cost && !resolution_supports_cost(resolution) {
        return Err(scene_director_critique(
            context,
            SceneDirectorFailureKind::Evidence,
            "cost beat is not backed by resolution outcome, gate, or visible effect",
            proposal.evidence_refs.clone(),
            vec!["Use a costly_success outcome, cost_imposed gate, or player-visible effect evidence.".to_owned()],
        ));
    }
    if proposal.beat_kind == DramaticBeatKind::Reveal && !resolution_supports_reveal(resolution) {
        return Err(scene_director_critique(
            context,
            SceneDirectorFailureKind::Evidence,
            "reveal beat is not backed by resolution evidence or visible belief/world update",
            proposal.evidence_refs.clone(),
            vec!["Ground reveal beats in resolution outcome/effects evidence.".to_owned()],
        ));
    }
    Ok(())
}

fn resolution_supports_cost(resolution: &ResolutionProposal) -> bool {
    matches!(
        resolution.outcome.kind,
        ResolutionOutcomeKind::CostlySuccess
    ) || resolution
        .gate_results
        .iter()
        .any(|gate| gate.status == GateStatus::CostImposed)
        || resolution.proposed_effects.iter().any(|effect| {
            effect.visibility == ResolutionVisibility::PlayerVisible
                && !effect.evidence_refs.is_empty()
                && effect.summary.contains("비용")
        })
}

fn resolution_supports_reveal(resolution: &ResolutionProposal) -> bool {
    !resolution.outcome.evidence_refs.is_empty()
        && (resolution.outcome.summary.contains("드러")
            || resolution.outcome.summary.contains("밝혀")
            || resolution
                .outcome
                .summary
                .to_ascii_lowercase()
                .contains("reveal"))
        || resolution.proposed_effects.iter().any(|effect| {
            effect.visibility == ResolutionVisibility::PlayerVisible
                && matches!(
                    effect.effect_kind,
                    crate::resolution::ProposedEffectKind::BeliefDelta
                        | crate::resolution::ProposedEffectKind::WorldLoreDelta
                )
                && !effect.evidence_refs.is_empty()
        })
}

fn audit_scene_director_choice_strategy(
    context: &PromptContextPacket,
    proposal: &SceneDirectorProposal,
    next_choices: &[crate::models::TurnChoice],
) -> std::result::Result<(), Box<SceneDirectorCritique>> {
    if !proposal.choice_strategy.must_change_choice_shape {
        return Ok(());
    }
    let rejected = next_choices
        .iter()
        .filter(|choice| {
            proposal
                .choice_strategy
                .avoid_recent_choice_tags
                .iter()
                .any(|tag| choice.tag.contains(tag) || tag.contains(choice.tag.as_str()))
        })
        .map(|choice| choice.tag.clone())
        .collect::<Vec<_>>();
    if rejected.is_empty() {
        return Ok(());
    }
    Err(scene_director_critique(
        context,
        SceneDirectorFailureKind::Repetition,
        "choice_strategy requested a changed choice shape but next_choices repeat avoided tags",
        rejected,
        vec!["Change visible choice tags/intents or clear must_change_choice_shape.".to_owned()],
    ))
}

fn collect_visible_strings(value: &Value) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    collect_visible_strings_into(value, &mut refs);
    refs
}

fn collect_visible_strings_into(value: &Value, refs: &mut BTreeSet<String>) {
    match value {
        Value::String(text) if !text.trim().is_empty() => {
            refs.insert(text.to_owned());
        }
        Value::Array(items) => {
            for item in items {
                collect_visible_strings_into(item, refs);
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                collect_visible_strings_into(item, refs);
            }
        }
        _ => {}
    }
}

fn contains_hidden_context_text(
    context: &PromptContextPacket,
    proposal: &SceneDirectorProposal,
) -> bool {
    let Ok(proposal_text) = serde_json::to_string(proposal) else {
        return true;
    };
    let Ok(hidden_context) = serde_json::to_value(&context.adjudication_context) else {
        return true;
    };
    collect_visible_strings(&hidden_context)
        .into_iter()
        .filter(|text| text.chars().count() >= 8)
        .any(|text| proposal_text.contains(text.as_str()))
}

fn scene_director_critique(
    context: &PromptContextPacket,
    failure_kind: SceneDirectorFailureKind,
    message: impl Into<String>,
    rejected_refs: Vec<String>,
    required_changes: Vec<String>,
) -> Box<SceneDirectorCritique> {
    Box::new(SceneDirectorCritique {
        schema_version: "singulari.scene_director_critique.v1".to_owned(),
        world_id: context.world_id.clone(),
        turn_id: context.turn_id.clone(),
        failure_kind,
        message: message.into(),
        rejected_refs,
        required_changes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_pressure::{
        SCENE_PRESSURE_PACKET_SCHEMA_VERSION, SCENE_PRESSURE_SCHEMA_VERSION, ScenePressurePolicy,
        ScenePressureProseEffect, ScenePressureVisibility,
    };
    use serde_json::json;

    #[test]
    fn compiles_high_tension_advisory_from_visible_pressure() {
        let packet = compile_scene_director_packet(
            "world",
            "turn_0003",
            &ScenePressurePacket {
                schema_version: SCENE_PRESSURE_PACKET_SCHEMA_VERSION.to_owned(),
                world_id: "world".to_owned(),
                turn_id: "turn_0003".to_owned(),
                visible_active: vec![ScenePressure {
                    schema_version: SCENE_PRESSURE_SCHEMA_VERSION.to_owned(),
                    pressure_id: "pressure:threat:torch".to_owned(),
                    kind: ScenePressureKind::Threat,
                    visibility: ScenePressureVisibility::PlayerVisible,
                    intensity: 4,
                    urgency: ScenePressureUrgency::Immediate,
                    source_refs: vec!["turn:0002".to_owned()],
                    observable_signals: vec!["torchlight closes in".to_owned()],
                    choice_affordances: vec!["move before seen".to_owned()],
                    prose_effect: ScenePressureProseEffect {
                        paragraph_pressure: "tight".to_owned(),
                        sensory_focus: vec!["light".to_owned()],
                        dialogue_style: "short".to_owned(),
                    },
                }],
                hidden_adjudication_only: Vec::new(),
                compiler_policy: ScenePressurePolicy::default(),
            },
            &json!({"active_patterns": []}),
        );

        assert_eq!(packet.current_scene.current_tension, TensionLevel::High);
        assert_eq!(packet.current_scene.scene_phase, ScenePhase::Crisis);
        assert!(
            packet
                .recommended_next_beats
                .iter()
                .all(|recommendation| recommendation.advisory_only)
        );
        assert!(
            packet
                .current_scene
                .dominant_pressure_refs
                .contains(&"pressure:threat:torch".to_owned())
        );
    }

    #[test]
    fn pattern_debt_adds_transition_and_repetition_hints() {
        let packet = compile_scene_director_packet(
            "world",
            "turn_0004",
            &ScenePressurePacket {
                schema_version: SCENE_PRESSURE_PACKET_SCHEMA_VERSION.to_owned(),
                world_id: "world".to_owned(),
                turn_id: "turn_0004".to_owned(),
                visible_active: vec![ScenePressure {
                    schema_version: SCENE_PRESSURE_SCHEMA_VERSION.to_owned(),
                    pressure_id: "pressure:time:horn".to_owned(),
                    kind: ScenePressureKind::TimePressure,
                    visibility: ScenePressureVisibility::PlayerVisible,
                    intensity: 4,
                    urgency: ScenePressureUrgency::Crisis,
                    source_refs: vec!["turn:0003".to_owned()],
                    observable_signals: vec!["horn is about to sound".to_owned()],
                    choice_affordances: vec!["choose quickly".to_owned()],
                    prose_effect: ScenePressureProseEffect {
                        paragraph_pressure: "urgent".to_owned(),
                        sensory_focus: vec!["sound".to_owned()],
                        dialogue_style: "cut off".to_owned(),
                    },
                }],
                hidden_adjudication_only: Vec::new(),
                compiler_policy: ScenePressurePolicy::default(),
            },
            &json!({
                "active_patterns": [{
                    "pattern_id": "scene_beat:pressure_tightens",
                    "surface": "scene_beat",
                    "recent_count": 3,
                    "replacement_pressure": ["change the problem shape"]
                }]
            }),
        );

        assert_eq!(
            packet.pacing_state.transition_pressure,
            TransitionPressure::High
        );
        assert_eq!(
            packet.current_scene.scene_phase,
            ScenePhase::TransitionReady
        );
        assert!(
            packet
                .recommended_next_beats
                .iter()
                .any(|recommendation| recommendation.beat_kind == DramaticBeatKind::Transition)
        );
        assert!(
            packet
                .forbidden_repetition
                .iter()
                .any(|hint| hint.contains("scene_beat:pressure_tightens"))
        );
    }

    #[test]
    fn broadened_inputs_influence_scene_director_recommendations() {
        let pressure = sample_pressure_packet(
            ScenePressureKind::Knowledge,
            "pressure:knowledge:mark",
            2,
            ScenePressureUrgency::Soon,
        );
        let packet = compile_scene_director_packet_from_input(SceneDirectorCompileInput {
            world_id: "world",
            turn_id: "turn_0005",
            scene_pressure: &pressure,
            active_pattern_debt: &json!({"active_patterns": []}),
            active_plot_threads: Some(&json!({
                "active_visible": [{
                    "thread_id": "plot_thread:gate_mark",
                    "thread_kind": "mystery",
                    "current_question": "What does the gate mark prove?"
                }]
            })),
            active_actor_agency: Some(&json!({
                "active_goals": [{
                    "actor_ref": "char:watcher",
                    "current_leverage": ["blocks the witness"],
                    "pressure_refs": ["pressure:knowledge:mark"]
                }],
                "recent_moves": []
            })),
            active_world_process_clock: Some(&json!({
                "visible_processes": [{
                    "process_id": "process:gate_closing",
                    "tempo": "immediate",
                    "source_refs": ["pressure:knowledge:mark"]
                }]
            })),
            active_player_intent_trace: Some(&json!({
                "active_intents": [{
                    "intent_shape": "inspect",
                    "evidence": "단서를 확인한다"
                }]
            })),
            active_social_exchange: None,
            recent_scene_window: Some(&json!(["표식을 살핀다.", "다시 묻는다."])),
        });

        assert_eq!(
            packet.current_scene.scene_question,
            "What does the gate mark prove?"
        );
        assert!(
            packet
                .current_scene
                .actor_refs
                .contains(&"char:watcher".to_owned())
        );
        assert!(
            packet
                .current_scene
                .unresolved_visible_threads
                .contains(&"plot_thread:gate_mark".to_owned())
        );
        assert_eq!(
            packet.pacing_state.transition_pressure,
            TransitionPressure::High
        );
        assert!(
            packet
                .recommended_next_beats
                .iter()
                .any(|recommendation| recommendation.beat_kind == DramaticBeatKind::Transition)
        );
    }

    #[test]
    fn materializes_scene_director_events_into_recent_beats() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let packet = compile_scene_director_packet(
            "world",
            "turn_0006",
            &sample_pressure_packet(
                ScenePressureKind::Threat,
                "pressure:threat:torch",
                4,
                ScenePressureUrgency::Immediate,
            ),
            &json!({"active_patterns": []}),
        );
        let proposal = SceneDirectorProposal {
            schema_version: SCENE_DIRECTOR_PROPOSAL_SCHEMA_VERSION.to_owned(),
            world_id: "world".to_owned(),
            turn_id: "turn_0006".to_owned(),
            scene_id: packet.current_scene.scene_id.clone(),
            beat_kind: DramaticBeatKind::Cost,
            turn_function: "Make the pressure cost something visible.".to_owned(),
            tension_before: TensionLevel::High,
            tension_after: TensionLevel::High,
            scene_effect: SceneEffect::CostImposed,
            paragraph_strategy: ParagraphStrategy {
                opening_shape: "action_consequence".to_owned(),
                middle_shape: "cost_tradeoff".to_owned(),
                closure_shape: "forced_next_decision".to_owned(),
            },
            choice_strategy: ChoiceStrategy {
                must_change_choice_shape: true,
                avoid_recent_choice_tags: Vec::new(),
            },
            transition: None,
            evidence_refs: vec!["turn:0005".to_owned()],
        };
        let plan = prepare_scene_director_event_plan(&packet, Some(&proposal))?;
        append_scene_director_event_plan(temp.path(), &plan)?;
        let rebuilt = rebuild_scene_director(temp.path(), &packet)?;

        assert_eq!(rebuilt.recent_beats.len(), 1);
        assert_eq!(rebuilt.recent_beats[0].beat_kind, DramaticBeatKind::Cost);
        assert!(temp.path().join(SCENE_DIRECTOR_FILENAME).is_file());
        Ok(())
    }

    #[test]
    fn preserves_active_scene_identity_across_fresh_compiles() {
        let first = compile_scene_director_packet(
            "world",
            "turn_0006",
            &sample_pressure_packet(
                ScenePressureKind::Threat,
                "pressure:threat:torch",
                4,
                ScenePressureUrgency::Immediate,
            ),
            &json!({"active_patterns": []}),
        );
        let mut history = first.clone();
        history.recent_beats = vec![DramaticBeat {
            beat_id: "scene_director_event:turn_0006:00".to_owned(),
            turn_id: "turn_0006".to_owned(),
            beat_kind: DramaticBeatKind::Escalate,
            turn_function: "Tighten the visible danger.".to_owned(),
            tension_before: TensionLevel::High,
            tension_after: TensionLevel::High,
            evidence_refs: vec!["turn:0005".to_owned()],
        }];
        let next = compile_scene_director_packet(
            "world",
            "turn_0007",
            &sample_pressure_packet(
                ScenePressureKind::Threat,
                "pressure:threat:torch",
                4,
                ScenePressureUrgency::Immediate,
            ),
            &json!({"active_patterns": []}),
        );
        assert_ne!(next.current_scene.scene_id, first.current_scene.scene_id);

        let merged = merge_scene_director_history(next, Some(&history));

        assert_eq!(merged.current_scene.scene_id, first.current_scene.scene_id);
        assert_eq!(merged.current_scene.opened_turn_id, "turn_0006");
        assert_eq!(merged.tuning_metrics.turns_in_current_scene, 2);
    }

    #[test]
    fn materializes_transition_as_next_active_scene() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let packet = compile_scene_director_packet(
            "world",
            "turn_0008",
            &sample_pressure_packet(
                ScenePressureKind::Knowledge,
                "pressure:knowledge:mark",
                3,
                ScenePressureUrgency::Soon,
            ),
            &json!({"active_patterns": []}),
        );
        let proposal = SceneDirectorProposal {
            schema_version: SCENE_DIRECTOR_PROPOSAL_SCHEMA_VERSION.to_owned(),
            world_id: "world".to_owned(),
            turn_id: "turn_0008".to_owned(),
            scene_id: packet.current_scene.scene_id.clone(),
            beat_kind: DramaticBeatKind::Transition,
            turn_function: "Move from the old clue to the next actionable question.".to_owned(),
            tension_before: TensionLevel::Medium,
            tension_after: TensionLevel::Medium,
            scene_effect: SceneEffect::SceneQuestionTransformed,
            paragraph_strategy: ParagraphStrategy {
                opening_shape: "new_information".to_owned(),
                middle_shape: "changed_problem".to_owned(),
                closure_shape: "new_scene_question".to_owned(),
            },
            choice_strategy: ChoiceStrategy {
                must_change_choice_shape: true,
                avoid_recent_choice_tags: Vec::new(),
            },
            transition: Some(SceneTransitionProposal {
                from_scene_id: packet.current_scene.scene_id.clone(),
                to_scene_question: "Who can act on the gate mark before dawn?".to_owned(),
                transition_reason: "The mark now points to a concrete actor and deadline."
                    .to_owned(),
                evidence_refs: vec!["turn:0007".to_owned()],
            }),
            evidence_refs: vec!["turn:0007".to_owned()],
        };
        let plan = prepare_scene_director_event_plan(&packet, Some(&proposal))?;
        append_scene_director_event_plan(temp.path(), &plan)?;
        let rebuilt = rebuild_scene_director(temp.path(), &packet)?;

        assert_eq!(
            rebuilt.current_scene.scene_question,
            "Who can act on the gate mark before dawn?"
        );
        assert_ne!(
            rebuilt.current_scene.scene_id,
            packet.current_scene.scene_id
        );
        assert_eq!(rebuilt.current_scene.opened_turn_id, "turn_0008");
        assert_eq!(rebuilt.current_scene.scene_phase, ScenePhase::Opening);
        Ok(())
    }

    fn sample_pressure_packet(
        kind: ScenePressureKind,
        pressure_id: &str,
        intensity: u8,
        urgency: ScenePressureUrgency,
    ) -> ScenePressurePacket {
        ScenePressurePacket {
            schema_version: SCENE_PRESSURE_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "world".to_owned(),
            turn_id: "turn_0005".to_owned(),
            visible_active: vec![ScenePressure {
                schema_version: SCENE_PRESSURE_SCHEMA_VERSION.to_owned(),
                pressure_id: pressure_id.to_owned(),
                kind,
                visibility: ScenePressureVisibility::PlayerVisible,
                intensity,
                urgency,
                source_refs: vec!["turn:0005".to_owned()],
                observable_signals: vec!["visible signal".to_owned()],
                choice_affordances: vec!["act on signal".to_owned()],
                prose_effect: ScenePressureProseEffect {
                    paragraph_pressure: "tight".to_owned(),
                    sensory_focus: vec!["signal".to_owned()],
                    dialogue_style: "short".to_owned(),
                },
            }],
            hidden_adjudication_only: Vec::new(),
            compiler_policy: ScenePressurePolicy::default(),
        }
    }
}
