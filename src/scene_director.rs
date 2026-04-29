use crate::scene_pressure::{
    ScenePressure, ScenePressureKind, ScenePressurePacket, ScenePressureUrgency,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SCENE_DIRECTOR_PACKET_SCHEMA_VERSION: &str = "singulari.scene_director_packet.v1";

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
    pub paragraph_budget_hint: ParagraphBudgetHint,
    pub compiler_policy: SceneDirectorPolicy,
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
    pub exit_conditions: Vec<SceneExitCondition>,
    #[serde(default)]
    pub unresolved_visible_threads: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
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
pub struct PacingState {
    #[serde(default)]
    pub recent_kind_sequence: Vec<DramaticBeatKind>,
    pub repeated_choice_shape_count: u8,
    pub repeated_closure_shape_count: u8,
    pub repeated_scene_beat_count: u8,
    pub high_tension_turns: u8,
    pub transition_pressure: TransitionPressure,
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

#[must_use]
pub fn compile_scene_director_packet(
    world_id: &str,
    turn_id: &str,
    scene_pressure: &ScenePressurePacket,
    active_pattern_debt: &Value,
) -> SceneDirectorPacket {
    let dominant_pressures = dominant_visible_pressures(scene_pressure);
    let current_tension = tension_from_pressures(&dominant_pressures);
    let repetition = repetition_counts(active_pattern_debt);
    let transition_pressure = transition_pressure(current_tension, repetition.scene_beat);
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
    let scene_question = scene_question(dominant_pressures.first().copied());
    let recent_beats = inferred_recent_beats(turn_id, current_tension, &dominant_pressures);
    let recommended_next_beats = recommendations(
        current_tension,
        transition_pressure,
        repetition.scene_beat,
        &dominant_pressure_refs,
    );
    let forbidden_repetition = forbidden_repetition(active_pattern_debt);

    SceneDirectorPacket {
        schema_version: SCENE_DIRECTOR_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        current_scene: SceneArc {
            scene_id: "scene:active".to_owned(),
            scene_question,
            scene_phase,
            opened_turn_id: turn_id.to_owned(),
            current_tension,
            dominant_pressure_refs,
            exit_conditions: exit_conditions(scene_phase),
            unresolved_visible_threads: Vec::new(),
            evidence_refs,
        },
        recent_beats,
        pacing_state: PacingState {
            recent_kind_sequence: Vec::new(),
            repeated_choice_shape_count: repetition.choice_shape,
            repeated_closure_shape_count: repetition.closure_shape,
            repeated_scene_beat_count: repetition.scene_beat,
            high_tension_turns: u8::from(matches!(current_tension, TensionLevel::High)),
            transition_pressure,
        },
        recommended_next_beats,
        forbidden_repetition,
        paragraph_budget_hint: paragraph_budget_hint(current_tension, transition_pressure),
        compiler_policy: SceneDirectorPolicy::default(),
    }
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

fn transition_pressure(tension: TensionLevel, repeated_scene_beat_count: u8) -> TransitionPressure {
    if repeated_scene_beat_count >= 3 && matches!(tension, TensionLevel::High) {
        TransitionPressure::High
    } else if repeated_scene_beat_count >= 2 || matches!(tension, TensionLevel::High) {
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

fn scene_question(strongest_pressure: Option<&ScenePressure>) -> String {
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

fn recommendations(
    tension: TensionLevel,
    transition_pressure: TransitionPressure,
    repeated_scene_beat_count: u8,
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
}
